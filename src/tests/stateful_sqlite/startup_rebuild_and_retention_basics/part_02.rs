#[tokio::test]
pub(crate) async fn proxy_only_working_conversation_live_aggregate_keeps_mixed_source_proxy_slice()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    for (invoke_id, source, seconds_ago, total_tokens, cost) in [
        ("mixed-proxy", SOURCE_PROXY, 30_i64, 120_i64, 0.12_f64),
        ("mixed-secondary", SOURCE_XY, 20_i64, 220_i64, 0.22_f64),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(invoke_id)
        .bind(format_naive(
            (now - ChronoDuration::seconds(seconds_ago))
                .with_timezone(&Shanghai)
                .naive_local(),
        ))
        .bind(source)
        .bind("success")
        .bind(total_tokens)
        .bind(cost)
        .bind(json!({ "promptCacheKey": "mixed-working-key" }).to_string())
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert mixed-source working conversation row");
    }

    let rows = query_prompt_cache_working_conversation_aggregates(
        &state.pool,
        &db_occurred_at_lower_bound(now - ChronoDuration::minutes(5)),
        InvocationSourceScope::ProxyOnly,
        10,
    )
    .await
    .expect("proxy-only working conversation aggregate should succeed");

    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.prompt_cache_key, "mixed-working-key");
    assert_eq!(row.request_count, 1);
    assert_eq!(row.total_tokens, 120);
    assert!((row.total_cost - 0.12).abs() < 1e-9);
}

#[tokio::test]
pub(crate) async fn health_check_reports_starting_until_startup_is_ready() {
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
pub(crate) async fn startup_hot_read_hydration_keeps_health_ready_under_sqlite_pool_pressure() {
    let (state, temp_dir, _db_url) = file_backed_test_state_with_busy_timeout(
        "startup-hot-read-readiness-pressure",
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .await;
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

    let hydration_handle =
        publish_http_readiness_and_spawn_hot_read_hydration_with_test_summary_deadline(
            state.clone(),
            Instant::now(),
            Duration::from_secs(4),
        );
    let health = tokio::time::timeout(
        Duration::from_millis(250),
        reqwest::get(format!("http://{addr}/health")),
    )
    .await
    .expect("health must not wait for SQLite hydration")
    .expect("health endpoint should respond while SQLite pool is saturated");
    assert_eq!(health.status(), StatusCode::OK);

    for endpoint in ["/api/stats/summary?window=today", "/api/system/status"] {
        let response = tokio::time::timeout(
            Duration::from_millis(250),
            reqwest::get(format!("http://{addr}{endpoint}")),
        )
        .await
        .expect("hot-read request must not wait for SQLite during startup")
        .expect("hot-read endpoint should return its unavailable contract");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(
        !hydration_handle.is_finished(),
        "the background worker retries after its bounded hydration attempt times out"
    );

    drop(held_connections);
    tokio::time::timeout(Duration::from_secs(5), hydration_handle)
        .await
        .expect("background startup hydration should recover after SQLite pressure clears")
        .expect("background startup hydration task should join");

    for endpoint in ["/api/stats/summary?window=today", "/api/system/status"] {
        let response = reqwest::get(format!("http://{addr}{endpoint}"))
            .await
            .expect("hydrated hot-read endpoint should respond");
        assert_eq!(response.status(), StatusCode::OK);
    }

    state.shutdown.cancel();
    server_handle
        .await
        .expect("http server should stop after startup pressure test");
    state.pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
pub(crate) async fn startup_summary_hydration_retries_after_an_in_flight_refresh() {
    let (state, temp_dir, _db_url) = file_backed_test_state_with_busy_timeout(
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
pub(crate) async fn startup_backfill_defers_legacy_mirror_work_until_cold_summary_is_published() {
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

async fn seed_startup_summary_legacy_mirror_fixture(state: &Arc<AppState>) -> String {
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
    archive_file_path
}

async fn assert_startup_summary_published_with_deferred_mirror(
    state: &Arc<AppState>,
    archive_file_path: &str,
) {
    let source_kind: String = sqlx::query_scalar(
        "SELECT summary_source_kind FROM archive_batches WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(archive_file_path)
    .fetch_one(&state.pool)
    .await
    .expect("load deferred archive source role");
    assert_eq!(source_kind, SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN);
    assert!(state.subscription_hub.summary_projection().await.is_some());
}

async fn assert_current_summary_is_unavailable_after_pool_close(state: &Arc<AppState>) {
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
}

#[tokio::test]
pub(crate) async fn startup_summary_hydration_publishes_before_legacy_mirror_recovery() {
    let (state, temp_dir, _db_url) = file_backed_test_state_with_busy_timeout(
        "startup-summary-legacy-mirror-recovery",
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .await;
    let archive_file_path = seed_startup_summary_legacy_mirror_fixture(&state).await;

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

    assert_startup_summary_published_with_deferred_mirror(&state, &archive_file_path).await;
    assert_current_summary_is_unavailable_after_pool_close(&state).await;

    state.shutdown.cancel();
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
pub(crate) async fn startup_system_status_hydrates_while_summary_hydration_is_delayed() {
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
pub(crate) async fn runtime_drain_joins_pending_startup_hydration_after_http_readiness() {
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
            RuntimeHandles {
                server_handle: Some(server_handle),
                startup_hot_read_hydration_handle: Some(hydration_handle),
                ..RuntimeHandles::default()
            },
        ),
    )
    .await
    .expect("runtime drain should not wait for abandoned startup hydration subwork")
    .expect("runtime drain should join the pending startup hydration coordinator");

    state.pool.close().await;
}

#[tokio::test]
pub(crate) async fn summary_startup_hydration_has_a_finite_sqlite_pressure_deadline() {
    let (state, temp_dir, _db_url) = file_backed_test_state_with_busy_timeout(
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

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        hydrate_summary_snapshots_with_deadline(state.as_ref(), Duration::from_secs(4)),
    )
    .await
    .expect("summary hydration must finish its bounded attempt under SQLite pressure")
    .expect_err("saturated SQLite pool should fail the bounded summary hydration attempt");
    assert!(
        result.to_string().contains("exceeded"),
        "summary hydration should report its finite build deadline: {result:#}"
    );

    drop(held_connections);
    state.pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
pub(crate) async fn system_status_startup_hydration_has_a_finite_sqlite_pressure_deadline() {
    let (state, temp_dir, _db_url) = file_backed_test_state_with_busy_timeout(
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

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        hydrate_system_status_snapshot(state.as_ref()),
    )
    .await
    .expect("system status hydration must finish its bounded attempt under SQLite pressure")
    .expect_err("saturated SQLite pool should fail the bounded system status hydration attempt");
    assert!(
        result.to_string().contains("exceeded"),
        "system status hydration should report its finite refresh deadline: {result:#}"
    );

    drop(held_connections);
    state.pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
pub(crate) async fn startup_hot_read_hydration_cancels_while_sqlite_pool_is_saturated() {
    let (state, temp_dir, _db_url) = file_backed_test_state_with_busy_timeout(
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

pub(crate) const HOURLY_ROLLUP_BOOTSTRAP_TASK_WAIT_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const HOURLY_ROLLUP_BOOTSTRAP_TASK_POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(crate) async fn wait_for_hourly_rollup_bootstrap_task(
    state: &AppState,
    expected_status: &str,
) -> (String, Option<String>, Option<String>) {
    let mut last_task = None;
    let result = tokio::time::timeout(HOURLY_ROLLUP_BOOTSTRAP_TASK_WAIT_TIMEOUT, async {
        loop {
            if expected_status != "running" {
                state
                    .sqlite_batch_writer
                    .flush_now(&state.pool)
                    .await
                    .expect("flush deferred hourly rollup bootstrap task-history writes");
            }
            let task = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
                r#"
                SELECT status, summary, detail
                FROM system_task_runs
                WHERE task_kind = 'hourly_rollup_bootstrap'
                ORDER BY id DESC
                LIMIT 1
                "#,
            )
            .fetch_optional(&state.pool)
            .await
            .expect("read hourly rollup bootstrap task");
            last_task.clone_from(&task);
            if let Some(task) = task
                && task.0 == expected_status
            {
                return task;
            }
            tokio::time::sleep(HOURLY_ROLLUP_BOOTSTRAP_TASK_POLL_INTERVAL).await;
        }
    })
    .await;
    if result.is_err() {
        let pending = state.sqlite_batch_writer.accounting_snapshot();
        let all_tasks = sqlx::query_as::<_, (i64, String, Option<String>, Option<String>)>(
            r#"
            SELECT id, status, summary, detail
            FROM system_task_runs
            WHERE task_kind = 'hourly_rollup_bootstrap'
            ORDER BY id DESC
            LIMIT 5
            "#,
        )
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();
        panic!(
            "hourly rollup bootstrap task should reach expected status {expected_status}; last_task={last_task:?}; all_tasks={all_tasks:?}; pending_depth={}; pending_bytes={}",
            pending.pending_depth, pending.pending_bytes,
        )
    }
    result.expect("hourly rollup bootstrap task should reach the expected state")
}

#[tokio::test]
pub(crate) async fn background_startup_hourly_rollup_bootstrap_keeps_health_ready_while_waiting_for_lock()
 {
    let state = test_state_from_config(test_config(), false).await;
    let (addr, server_handle) = spawn_http_server(state.clone())
        .await
        .expect("spawn http server");
    state.startup_ready.store(true, Ordering::Release);

    let rollup_guard = state.hourly_rollup_sync_lock.lock().await;
    let bootstrap_handle =
        spawn_runtime_startup_hourly_rollup_bootstrap(state.clone(), state.shutdown.clone());
    let (status, _, _) = wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "running").await;
    assert_eq!(status, "running");

    let health = reqwest::get(format!("http://{addr}/health"))
        .await
        .expect("health endpoint should respond while rollup bootstrap waits for the lock");
    assert_eq!(health.status(), StatusCode::OK);

    drop(rollup_guard);
    tokio::time::timeout(Duration::from_secs(5), bootstrap_handle)
        .await
        .expect("background bootstrap should finish")
        .expect("background bootstrap task should join");
    let (status, summary, detail) =
        wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "success").await;
    assert_eq!(status, "success");
    assert!(
        summary
            .as_deref()
            .is_some_and(|summary| summary.contains("hourly_rollups_ms="))
    );
    assert!(detail.is_none());

    state.shutdown.cancel();
    server_handle.await.expect("http server task should join");
}

#[tokio::test]
pub(crate) async fn background_startup_hourly_rollup_bootstrap_records_failure_without_revoking_readiness()
 {
    let state = test_state_from_config(test_config(), false).await;
    state.startup_ready.store(true, Ordering::Release);
    sqlx::query("DROP TABLE upstream_account_stats_hourly")
        .execute(&state.pool)
        .await
        .expect("remove account rollup table to force bootstrap failure");

    spawn_runtime_startup_hourly_rollup_bootstrap(state.clone(), state.shutdown.clone())
        .await
        .expect("background bootstrap task should join");

    let (status, summary, detail) =
        wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "failed").await;
    assert_eq!(status, "failed");
    assert!(
        summary
            .as_deref()
            .is_some_and(|summary| summary.contains("existing rollups remain available"))
    );
    assert!(
        detail.is_some(),
        "failure should retain an observable error"
    );
    assert_eq!(
        health_check(State(state.clone()))
            .await
            .into_response()
            .status(),
        StatusCode::OK,
        "background bootstrap failure must not return health to starting"
    );

    state.shutdown.cancel();
}

#[tokio::test]
pub(crate) async fn background_startup_hourly_rollup_bootstrap_cancels_while_waiting_for_lock() {
    let state = test_state_from_config(test_config(), false).await;
    let rollup_guard = state.hourly_rollup_sync_lock.lock().await;
    let bootstrap_handle =
        spawn_runtime_startup_hourly_rollup_bootstrap(state.clone(), state.shutdown.clone());
    wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "running").await;

    state.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), bootstrap_handle)
        .await
        .expect("background bootstrap should stop after cancellation")
        .expect("background bootstrap task should join");
    drop(rollup_guard);
    state
        .sqlite_batch_writer
        .flush_now(&state.pool)
        .await
        .expect("flush deferred cancellation task-history finish");

    let (status, summary, detail) =
        wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "skipped").await;
    assert_eq!(status, "skipped");
    assert!(
        summary
            .as_deref()
            .is_some_and(|summary| summary.contains("cancelled before acquiring"))
    );
    assert!(detail.is_none());
}

#[tokio::test]
pub(crate) async fn background_startup_hourly_rollup_bootstrap_cancels_while_task_history_start_is_locked()
 {
    let (state, temp_dir, db_url) = file_backed_test_state_with_busy_timeout(
        "startup-hourly-rollup-bootstrap-task-history-lock",
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .await;
    let mut lock_conn = SqliteConnection::connect(&db_url)
        .await
        .expect("connect task-history lock holder");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut lock_conn)
        .await
        .expect("acquire task-history write lock");

    let mut bootstrap_handle =
        spawn_runtime_startup_hourly_rollup_bootstrap(state.clone(), state.shutdown.clone());
    tokio::time::timeout(Duration::from_millis(100), &mut bootstrap_handle)
        .await
        .expect_err("bootstrap should wait for the task-history write lock");

    state.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), bootstrap_handle)
        .await
        .expect("background bootstrap should stop without waiting for sqlite busy timeout")
        .expect("background bootstrap task should join");

    sqlx::query("ROLLBACK")
        .execute(&mut lock_conn)
        .await
        .expect("release task-history write lock");
    let orphaned_running_tasks: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM system_task_runs
        WHERE task_kind = 'hourly_rollup_bootstrap'
          AND trigger_kind = 'startup'
          AND status = 'running'
          AND summary = 'background hourly rollup bootstrap started'
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("count cancelled bootstrap task-history rows");
    assert_eq!(orphaned_running_tasks, 0);
    state.pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
pub(crate) async fn background_startup_hourly_rollup_bootstrap_retries_coordinator_contention_promptly()
 {
    let state = test_state_from_config(test_config(), false).await;
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let write_permit = coordinator
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
        .await;
    let bootstrap_handle =
        spawn_runtime_startup_hourly_rollup_bootstrap(state.clone(), state.shutdown.clone());

    tokio::time::sleep(Duration::from_millis(100)).await;
    let task_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'hourly_rollup_bootstrap'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("read bootstrap task count while coordinator is occupied");
    assert_eq!(task_count, 0);

    drop(write_permit);
    wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "running").await;
    state.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), bootstrap_handle)
        .await
        .expect("bootstrap should stop after cancellation")
        .expect("bootstrap task should join after cancellation");
    state.pool.close().await;
}

#[tokio::test]
pub(crate) async fn background_startup_hourly_rollup_bootstrap_defers_task_history_finish_during_shutdown_lock()
 {
    let (state, temp_dir, db_url) = file_backed_test_state_with_busy_timeout(
        "startup-hourly-rollup-bootstrap-task-history-finish-lock",
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .await;
    let rollup_guard = state.hourly_rollup_sync_lock.lock().await;
    let bootstrap_handle =
        spawn_runtime_startup_hourly_rollup_bootstrap(state.clone(), state.shutdown.clone());
    wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "running").await;

    let mut lock_conn = SqliteConnection::connect(&db_url)
        .await
        .expect("connect task-history lock holder");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut lock_conn)
        .await
        .expect("acquire task-history write lock");

    state.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), bootstrap_handle)
        .await
        .expect("background bootstrap should defer its task-history finish during shutdown")
        .expect("background bootstrap task should join");
    drop(rollup_guard);

    sqlx::query("ROLLBACK")
        .execute(&mut lock_conn)
        .await
        .expect("release task-history write lock");
    state
        .sqlite_batch_writer
        .flush_now(&state.pool)
        .await
        .expect("flush deferred task-history finish");

    let (status, summary, detail) =
        wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "skipped").await;
    assert_eq!(status, "skipped");
    assert!(
        summary
            .as_deref()
            .is_some_and(|summary| summary.contains("cancelled before acquiring"))
    );
    assert!(detail.is_none());

    state.pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
pub(crate) async fn startup_hourly_rollup_task_history_finish_does_not_wait_for_write_lock() {
    let (state, temp_dir, db_url) = file_backed_test_state_with_busy_timeout(
        "startup-hourly-rollup-bootstrap-task-history-finish-cancel-race",
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .await;
    let task_run = begin_system_task_run(
        &state.pool,
        SystemTaskKind::HourlyRollupBootstrap,
        "startup",
        Some("background hourly rollup bootstrap started".to_string()),
    )
    .await
    .expect("record task-history start");
    let mut lock_conn = SqliteConnection::connect(&db_url)
        .await
        .expect("connect task-history lock holder");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut lock_conn)
        .await
        .expect("acquire task-history write lock");

    let cancel = CancellationToken::new();
    let state_for_finish = state.clone();
    let cancel_for_finish = cancel.clone();
    let finish_handle = tokio::spawn(async move {
        finish_runtime_startup_hourly_rollup_bootstrap_task(
            state_for_finish.as_ref(),
            &cancel_for_finish,
            Some(&task_run),
            SystemTaskStatus::Skipped,
            "background hourly rollup bootstrap cancelled before acquiring its synchronization lock",
            None,
        )
        .await;
    });
    tokio::time::timeout(Duration::from_millis(100), finish_handle)
        .await
        .expect("task-history finish should enqueue without waiting for the sqlite write lock")
        .expect("task-history finish task should join");

    sqlx::query("ROLLBACK")
        .execute(&mut lock_conn)
        .await
        .expect("release task-history write lock");
    state
        .sqlite_batch_writer
        .flush_now(&state.pool)
        .await
        .expect("flush deferred task-history finish");

    let (status, summary, detail) =
        wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "skipped").await;
    assert_eq!(status, "skipped");
    assert!(
        summary
            .as_deref()
            .is_some_and(|summary| summary.contains("cancelled before acquiring"))
    );
    assert!(detail.is_none());

    state.pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
pub(crate) async fn retention_run_once_keeps_blocking_hourly_rollup_bootstrap() {
    let state = test_state_from_config(test_config(), false).await;
    let cli = CliArgs {
        retention_run_once: true,
        ..CliArgs::default()
    };

    let summary = run_startup_persistent_prep(&state.pool, &state.config, &cli)
        .await
        .expect("retention run-once startup prep should finish");
    assert!(summary.bootstrapped_hourly_rollups);

    state.shutdown.cancel();
}

#[tokio::test]
pub(crate) async fn startup_backfill_progress_persists_terminal_missing_raw_cursor() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response, request_raw_path
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("reasoning-missing-raw")
    .bind("2026-03-09 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("{}")
    .bind("{}")
    .bind("missing-reasoning-request.json")
    .execute(&state.pool)
    .await
    .expect("insert reasoning backfill row");

    let row_id: i64 =
        sqlx::query_scalar("SELECT id FROM codex_invocations WHERE invoke_id = ?1 LIMIT 1")
            .bind("reasoning-missing-raw")
            .fetch_one(&state.pool)
            .await
            .expect("fetch inserted row id");

    run_startup_backfill_task_if_due(&state, StartupBackfillTask::ReasoningEffort)
        .await
        .expect("first startup backfill pass should succeed");

    let task_name =
        startup_backfill_task_progress_key(state.as_ref(), StartupBackfillTask::ReasoningEffort)
            .await;
    let progress = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load backfill progress after first pass");
    assert_eq!(progress.cursor_id, row_id);
    assert_eq!(progress.last_scanned, 1);
    assert_eq!(progress.last_updated, 0);
    assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_OK);

    sqlx::query("UPDATE startup_backfill_progress SET next_run_after = ?1 WHERE task_name = ?2")
        .bind(format_utc_iso(Utc::now() - ChronoDuration::seconds(1)))
        .bind(&task_name)
        .execute(&state.pool)
        .await
        .expect("force startup backfill task due again");

    run_startup_backfill_task_if_due(&state, StartupBackfillTask::ReasoningEffort)
        .await
        .expect("second startup backfill pass should skip previously scanned row");

    let progress = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load backfill progress after second pass");
    assert_eq!(progress.cursor_id, row_id);
    assert_eq!(progress.last_scanned, 0);
    assert_eq!(progress.last_updated, 0);
}

use super::*;
