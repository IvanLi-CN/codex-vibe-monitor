async fn spawn_maintenance_usage_server() -> (Arc<AtomicUsize>, tokio::task::JoinHandle<()>, String)
{
    async fn handler(State(requests): State<Arc<AtomicUsize>>) -> (StatusCode, String) {
        requests.fetch_add(1, Ordering::SeqCst);
        (
            StatusCode::OK,
            json!({
                "planType": "team",
                "rateLimit": {
                    "primaryWindow": {
                        "usedPercent": 8,
                        "windowDurationMins": 300,
                        "resetsAt": 1771322400
                    },
                    "secondaryWindow": {
                        "usedPercent": 8,
                        "windowDurationMins": 10080,
                        "resetsAt": 1771927200
                    }
                }
            })
            .to_string(),
        )
    }
    let requests = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/backend-api/wham/usage", get(handler))
        .with_state(requests.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind usage server");
    let addr = listener.local_addr().expect("usage server addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve usage server");
    });
    (requests, server, format!("http://{addr}/backend-api"))
}

#[tokio::test]
pub(crate) async fn maintenance_pass_waits_for_brief_background_busy_slot() {
    let (base_url, started, release, requests, server) = spawn_blocking_usage_server().await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Waited Maintenance OAuth",
        "waited-maintenance@example.com",
        "org_waited_maintenance",
        "user_waited_maintenance",
    )
    .await;

    let gate = Arc::new(crate::db_pressure::DbPressureGate::new(
        1,
        Duration::from_secs(1),
    ));
    let held = gate
        .try_begin_background("startup_backfill")
        .expect("hold background slot");
    let pass = tokio::spawn({
        let gate = gate.clone();
        let state = state.clone();
        async move {
            run_upstream_account_maintenance_once_with_gate(
                state,
                gate.as_ref(),
                Duration::from_millis(500),
            )
            .await
        }
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !started.load(Ordering::SeqCst),
        "maintenance sync should wait while the only background slot is busy"
    );

    drop(held);
    pass.await
        .expect("maintenance pass should not panic")
        .expect("maintenance pass should dispatch after the slot is released");

    wait_for_atomic_true(started.as_ref()).await;
    release.notify_waiters();
    timeout(Duration::from_secs(1), async {
        while requests.load(Ordering::SeqCst) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("queued maintenance request should complete");
    server.abort();
}

#[tokio::test]
pub(crate) async fn drain_background_tasks_waits_for_queued_maintenance_syncs() {
    let (base_url, started, release, _requests, server) = spawn_blocking_usage_server().await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Drain Maintenance OAuth",
        "drain-maintenance@example.com",
        "org_drain_maintenance",
        "user_drain_maintenance",
    )
    .await;

    run_upstream_account_maintenance_once(state.clone())
        .await
        .expect("maintenance pass should dispatch");
    wait_for_atomic_true(started.as_ref()).await;

    let mut drain_task = tokio::spawn({
        let runtime = state.upstream_accounts.clone();
        async move {
            runtime.drain_background_tasks().await;
        }
    });
    assert!(
        timeout(Duration::from_millis(150), &mut drain_task)
            .await
            .is_err(),
        "drain should wait for queued maintenance tasks"
    );

    release.notify_waiters();
    drain_task.await.expect("drain join should succeed");
    server.abort();
}

#[tokio::test]
pub(crate) async fn maintenance_pass_reconciles_legacy_upstream_rejected_cooldown_rows() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Legacy Rejected Cooldown OAuth",
        "legacy-rejected@example.com",
        "org_legacy_rejected",
        "user_legacy_rejected",
    )
    .await;
    let failed_at = Utc::now() - ChronoDuration::hours(1);
    let failed_at_iso = format_utc_iso(failed_at);

    sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET status = ?2,
                last_error = ?3,
                last_error_at = ?4,
                last_route_failure_at = ?4,
                last_route_failure_kind = ?5,
                last_action = ?6,
                last_action_source = ?7,
                last_action_reason_code = ?8,
                last_action_reason_message = ?9,
                last_action_http_status = 402,
                last_action_at = ?4,
                cooldown_until = NULL
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(UPSTREAM_ACCOUNT_STATUS_ERROR)
        .bind("usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}")
        .bind(&failed_at_iso)
        .bind(PROXY_FAILURE_UPSTREAM_HTTP_402)
        .bind(UPSTREAM_ACCOUNT_ACTION_SYNC_HARD_UNAVAILABLE)
        .bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE)
        .bind("upstream_http_402")
        .bind("usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}")
        .execute(&state.pool)
        .await
        .expect("seed legacy maintenance rejected row");

    run_upstream_account_maintenance_once(state.clone())
        .await
        .expect("run maintenance pass");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load reconciled row")
        .expect("reconciled row exists");
    assert_eq!(
        after.cooldown_until,
        Some(format_utc_iso(
            failed_at
                + ChronoDuration::seconds(
                    UPSTREAM_ACCOUNT_UPSTREAM_REJECTED_MAINTENANCE_COOLDOWN_SECS,
                ),
        ))
    );
    assert_eq!(after.last_synced_at, None);
}

#[tokio::test]
pub(crate) async fn maintenance_dispatch_respects_parallelism_limit() {
    async fn handler(
        State((requests, release)): State<(Arc<AtomicUsize>, Arc<Semaphore>)>,
    ) -> (StatusCode, String) {
        requests.fetch_add(1, Ordering::SeqCst);
        let _permit = release
            .acquire()
            .await
            .expect("test release semaphore should stay open");
        (
            StatusCode::OK,
            json!({
                "planType": "team",
                "rateLimit": {
                    "primaryWindow": {
                        "usedPercent": 8,
                        "windowDurationMins": 300,
                        "resetsAt": 1771322400
                    }
                }
            })
            .to_string(),
        )
    }

    let requests = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(Semaphore::new(0));
    let app = Router::new()
        .route("/backend-api/wham/usage", get(handler))
        .with_state((requests.clone(), release.clone()));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind bounded usage server");
    let addr = listener.local_addr().expect("bounded usage server addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve bounded usage server");
    });

    let state =
        test_app_state_with_usage_base_and_parallelism(&format!("http://{addr}/backend-api"), 1)
            .await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Bounded Maintenance A",
        "bounded-a@example.com",
        "org_bounded_a",
        "user_bounded_a",
    )
    .await;
    insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Bounded Maintenance B",
        "bounded-b@example.com",
        "org_bounded_b",
        "user_bounded_b",
    )
    .await;

    run_upstream_account_maintenance_once(state.clone())
        .await
        .expect("dispatch maintenance pass");
    wait_for_atomic_usize(requests.as_ref(), 1).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        requests.load(Ordering::SeqCst),
        1,
        "only one maintenance sync should reach the upstream at a time"
    );

    release.add_permits(1);
    wait_for_atomic_usize(requests.as_ref(), 2).await;
    release.add_permits(1);
    server.abort();
}

#[tokio::test]
pub(crate) async fn maintenance_sync_does_not_block_unrelated_account_updates() {
    let (base_url, started, release, requests, server) = spawn_blocking_usage_server().await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let maintenance_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Maintenance OAuth",
        "maintenance@example.com",
        "org_maintenance",
        "user_maintenance",
    )
    .await;
    let updated_account_id = insert_api_key_account(&state.pool, "Unrelated API Key").await;

    let maintenance_task = tokio::spawn({
        let state = state.clone();
        async move {
            state
                .upstream_accounts
                .account_ops
                .run_maintenance_sync(state.clone(), maintenance_account_id)
                .await
        }
    });
    wait_for_atomic_true(started.as_ref()).await;

    let started_at = std::time::Instant::now();
    state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            updated_account_id,
            UpdateUpstreamAccountRequest {
                display_name: None,
                email: OptionalField::Missing,
                group_name: None,
                group_bound_proxy_keys: None,
                group_node_shunt_enabled: None,
                group_single_account_rotation_enabled: None,
                note: Some("updated while maintenance runs".to_string()),
                group_note: None,
                concurrency_limit: None,
                upstream_base_url: OptionalField::Missing,
                bound_proxy_keys: OptionalField::Missing,
                enabled: Some(false),
                is_mother: None,
                api_key: None,
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: None,
                routing_rule: None,
                ..UpdateUpstreamAccountRequest::default()
            },
        )
        .await
        .expect("update unrelated account");
    assert!(
        started_at.elapsed() < Duration::from_secs(1),
        "unrelated account update should not wait for maintenance"
    );

    let updated_row = load_upstream_account_row(&state.pool, updated_account_id)
        .await
        .expect("load updated account")
        .expect("updated account exists");
    assert_eq!(updated_row.enabled, 0);
    assert_eq!(
        updated_row.note.as_deref(),
        Some("updated while maintenance runs")
    );

    release.notify_waiters();
    assert_eq!(
        maintenance_task
            .await
            .expect("maintenance join")
            .expect("maintenance result"),
        MaintenanceDispatchOutcome::Executed
    );
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
pub(crate) async fn persist_imported_oauth_waits_for_inflight_maintenance() {
    let (base_url, started, release, _requests, server) = spawn_blocking_usage_server().await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Imported OAuth",
        "imported@example.com",
        "org_imported",
        "user_imported",
    )
    .await;

    let maintenance_task = tokio::spawn({
        let state = state.clone();
        async move {
            state
                .upstream_accounts
                .account_ops
                .run_maintenance_sync(state.clone(), account_id)
                .await
        }
    });
    wait_for_atomic_true(started.as_ref()).await;

    let probe = ImportedOauthProbeOutcome {
        token_expires_at: format_utc_iso(Utc::now() + ChronoDuration::days(30)),
        credentials: StoredOauthCredentials {
            access_token: "imported-access-token".to_string(),
            refresh_token: Some("imported-refresh-token".to_string()),
            id_token: test_id_token(
                "imported@example.com",
                Some("org_imported"),
                Some("user_imported"),
                Some("team"),
            ),
            token_type: Some("Bearer".to_string()),
        },
        claims: test_claims(
            "imported@example.com",
            Some("org_imported"),
            Some("user_imported"),
        ),
        usage_snapshot: None,
        maintenance_proxy_snapshot: None,
        exhausted: false,
        usage_snapshot_warning: Some("usage snapshot unavailable".to_string()),
    };

    let mut import_task = tokio::spawn({
        let state = state.clone();
        async move {
            state
                .upstream_accounts
                .account_ops
                .run_persist_imported_oauth(state.clone(), account_id, probe)
                .await
        }
    });
    assert!(
        timeout(Duration::from_millis(150), &mut import_task)
            .await
            .is_err(),
        "post-import updates should queue behind same-account maintenance"
    );

    release.notify_waiters();
    assert_eq!(
        maintenance_task
            .await
            .expect("maintenance join")
            .expect("maintenance result"),
        MaintenanceDispatchOutcome::Executed
    );
    assert_eq!(
        import_task
            .await
            .expect("import join")
            .expect("persist imported oauth"),
        Some("usage snapshot unavailable".to_string())
    );
    server.abort();
}

#[tokio::test]
pub(crate) async fn same_account_updates_wait_for_inflight_maintenance() {
    let (base_url, started, release, _requests, server) = spawn_blocking_usage_server().await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Serialized OAuth",
        "serialized@example.com",
        "org_serialized",
        "user_serialized",
    )
    .await;

    let maintenance_task = tokio::spawn({
        let state = state.clone();
        async move {
            state
                .upstream_accounts
                .account_ops
                .run_maintenance_sync(state.clone(), account_id)
                .await
        }
    });
    wait_for_atomic_true(started.as_ref()).await;

    let mut update_task = tokio::spawn({
        let state = state.clone();
        async move {
            state
                .upstream_accounts
                .account_ops
                .run_update_account(
                    state.clone(),
                    account_id,
                    UpdateUpstreamAccountRequest {
                        display_name: None,
                        email: OptionalField::Missing,
                        group_name: None,
                        group_bound_proxy_keys: None,
                        group_node_shunt_enabled: None,
                        group_single_account_rotation_enabled: None,
                        note: Some("queued note".to_string()),
                        group_note: None,
                        concurrency_limit: None,
                        upstream_base_url: OptionalField::Missing,
                        bound_proxy_keys: OptionalField::Missing,
                        enabled: None,
                        is_mother: None,
                        api_key: None,
                        local_primary_limit: None,
                        local_secondary_limit: None,
                        local_limit_unit: None,
                        tag_ids: None,
                        routing_rule: None,
                        ..UpdateUpstreamAccountRequest::default()
                    },
                )
                .await
        }
    });
    assert!(
        timeout(Duration::from_millis(150), &mut update_task)
            .await
            .is_err(),
        "same-account update should queue behind maintenance"
    );

    let row_during_maintenance = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load row during maintenance")
        .expect("row exists");
    assert_eq!(row_during_maintenance.note, None);

    release.notify_waiters();
    assert_eq!(
        maintenance_task
            .await
            .expect("maintenance join")
            .expect("maintenance result"),
        MaintenanceDispatchOutcome::Executed
    );
    update_task
        .await
        .expect("update join")
        .expect("update result");

    let updated_row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load updated row")
        .expect("updated row exists");
    assert_eq!(updated_row.note.as_deref(), Some("queued note"));
    server.abort();
}

#[tokio::test]
pub(crate) async fn maintenance_sync_deduplicates_same_account_work() {
    let (base_url, started, release, requests, server) = spawn_blocking_usage_server().await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Deduped OAuth",
        "deduped@example.com",
        "org_deduped",
        "user_deduped",
    )
    .await;

    let maintenance_task = tokio::spawn({
        let state = state.clone();
        async move {
            state
                .upstream_accounts
                .account_ops
                .run_maintenance_sync(state.clone(), account_id)
                .await
        }
    });
    wait_for_atomic_true(started.as_ref()).await;

    let second = state
        .upstream_accounts
        .account_ops
        .run_maintenance_sync(state.clone(), account_id)
        .await
        .expect("second maintenance result");
    assert_eq!(second, MaintenanceDispatchOutcome::Deduped);

    release.notify_waiters();
    assert_eq!(
        maintenance_task
            .await
            .expect("maintenance join")
            .expect("maintenance result"),
        MaintenanceDispatchOutcome::Executed
    );
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
pub(crate) async fn queued_maintenance_sync_revalidates_due_window_before_execution() {
    let (requests, server, usage_base_url) = spawn_maintenance_usage_server().await;
    let state = test_app_state_with_usage_base_and_parallelism(&usage_base_url, 1).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Queued Revalidation OAuth",
        "queued-revalidation@example.com",
        "org_queued_revalidation",
        "user_queued_revalidation",
    )
    .await;
    insert_limit_sample_with_usage(
        &state.pool,
        account_id,
        "2026-03-23T11:00:00Z",
        Some(12.0),
        Some(10.0),
    )
    .await;

    let due_at = format_utc_iso(Utc::now() - ChronoDuration::minutes(10));
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET last_synced_at = ?2,
                last_successful_sync_at = ?2
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind(&due_at)
    .execute(&state.pool)
    .await
    .expect("seed due sync time");

    let held_slot = state
        .upstream_accounts
        .account_ops
        .maintenance_slots
        .clone()
        .acquire_owned()
        .await
        .expect("hold maintenance slot");
    assert_eq!(
        state
            .upstream_accounts
            .account_ops
            .dispatch_maintenance_sync(
                state.clone(),
                MaintenanceDispatchPlan {
                    account_id,
                    tier: MaintenanceTier::Priority,
                    sync_interval_secs: 300,
                },
            )
            .expect("queue maintenance plan"),
        MaintenanceQueueOutcome::Queued
    );

    let fresh_sync_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET last_synced_at = ?2,
                last_successful_sync_at = ?2
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind(&fresh_sync_at)
    .execute(&state.pool)
    .await
    .expect("refresh sync time before queued maintenance executes");

    drop(held_slot);
    state.upstream_accounts.drain_background_tasks().await;

    assert_eq!(
        requests.load(Ordering::SeqCst),
        0,
        "queued maintenance should skip once a newer sync makes the plan stale"
    );
    server.abort();
}

#[tokio::test]
pub(crate) async fn queued_high_frequency_maintenance_sync_revalidates_current_tier_before_execution()
 {
    let (requests, server, usage_base_url) = spawn_maintenance_usage_server().await;
    let state = test_app_state_with_usage_base_and_parallelism(&usage_base_url, 1).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Queued High Frequency OAuth",
        "queued-high-frequency@example.com",
        "org_queued_high_frequency",
        "user_queued_high_frequency",
    )
    .await;
    insert_limit_sample_with_usage(
        &state.pool,
        account_id,
        "2026-03-23T11:00:00Z",
        Some(12.0),
        Some(10.0),
    )
    .await;

    let due_at = format_utc_iso(Utc::now() - ChronoDuration::seconds(70));
    let working_selected_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET last_synced_at = ?2,
                last_successful_sync_at = ?2,
                last_selected_at = ?3
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind(&due_at)
    .bind(&working_selected_at)
    .execute(&state.pool)
    .await
    .expect("seed high-frequency due account");

    let held_slot = state
        .upstream_accounts
        .account_ops
        .maintenance_slots
        .clone()
        .acquire_owned()
        .await
        .expect("hold maintenance slot");
    assert_eq!(
        state
            .upstream_accounts
            .account_ops
            .dispatch_maintenance_sync(
                state.clone(),
                MaintenanceDispatchPlan {
                    account_id,
                    tier: MaintenanceTier::HighFrequency,
                    sync_interval_secs: MIN_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS,
                },
            )
            .expect("queue maintenance plan"),
        MaintenanceQueueOutcome::Queued
    );

    let idle_selected_at = format_utc_iso(
        Utc::now() - ChronoDuration::minutes(POOL_ROUTE_ACTIVE_STICKY_WINDOW_MINUTES + 1),
    );
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET last_selected_at = ?2
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind(&idle_selected_at)
    .execute(&state.pool)
    .await
    .expect("drop account out of high-frequency tier before queued maintenance executes");

    drop(held_slot);
    state.upstream_accounts.drain_background_tasks().await;

    assert_eq!(
        requests.load(Ordering::SeqCst),
        0,
        "queued high-frequency maintenance should skip once the account falls back to a slower tier"
    );
    server.abort();
}

async fn seed_priority_fallback_accounts(state: &AppState) -> (i64, i64) {
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Queued High Frequency Re-rank OAuth",
        "queued-high-frequency-rerank@example.com",
        "org_queued_high_frequency_rerank",
        "user_queued_high_frequency_rerank",
    )
    .await;
    insert_limit_sample_with_usage(
        &state.pool,
        account_id,
        "2026-03-23T11:00:00Z",
        Some(20.0),
        Some(20.0),
    )
    .await;
    let competing_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Competing Priority OAuth",
        "competing-priority@example.com",
        "org_competing_priority",
        "user_competing_priority",
    )
    .await;
    insert_limit_sample_with_usage(
        &state.pool,
        competing_account_id,
        "2026-03-23T11:00:00Z",
        Some(5.0),
        Some(5.0),
    )
    .await;
    (account_id, competing_account_id)
}

#[tokio::test]
pub(crate) async fn queued_high_frequency_maintenance_sync_recomputes_priority_fallback_before_execution()
 {
    let (requests, server, usage_base_url) = spawn_maintenance_usage_server().await;
    let state = test_app_state_with_usage_base_and_parallelism(&usage_base_url, 1).await;
    let (account_id, competing_account_id) = seed_priority_fallback_accounts(&state).await;

    let due_at = format_utc_iso(Utc::now() - ChronoDuration::seconds(400));
    let working_selected_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET last_synced_at = ?2,
                last_successful_sync_at = ?2,
                last_selected_at = ?3
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind(&due_at)
    .bind(&working_selected_at)
    .execute(&state.pool)
    .await
    .expect("seed due high-frequency account");

    let competing_synced_at = format_utc_iso(Utc::now() - ChronoDuration::seconds(120));
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET last_synced_at = ?2,
                last_successful_sync_at = ?2
            WHERE id = ?1
            "#,
    )
    .bind(competing_account_id)
    .bind(&competing_synced_at)
    .execute(&state.pool)
    .await
    .expect("seed competing account sync time");

    let held_slot = state
        .upstream_accounts
        .account_ops
        .maintenance_slots
        .clone()
        .acquire_owned()
        .await
        .expect("hold maintenance slot");
    assert_eq!(
        state
            .upstream_accounts
            .account_ops
            .dispatch_maintenance_sync(
                state.clone(),
                MaintenanceDispatchPlan {
                    account_id,
                    tier: MaintenanceTier::HighFrequency,
                    sync_interval_secs: MIN_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS,
                },
            )
            .expect("queue maintenance plan"),
        MaintenanceQueueOutcome::Queued
    );

    let idle_selected_at = format_utc_iso(
        Utc::now() - ChronoDuration::minutes(POOL_ROUTE_ACTIVE_STICKY_WINDOW_MINUTES + 1),
    );
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET last_selected_at = ?2
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind(&idle_selected_at)
    .execute(&state.pool)
    .await
    .expect("drop queued account out of high-frequency tier");

    sqlx::query(
        r#"
            UPDATE pool_upstream_account_limit_samples
            SET primary_used_percent = 100.0,
                secondary_used_percent = 100.0
            WHERE account_id = ?1
            "#,
    )
    .bind(competing_account_id)
    .execute(&state.pool)
    .await
    .expect("remove competing account from available priority competition");

    drop(held_slot);
    state.upstream_accounts.drain_background_tasks().await;

    assert_eq!(requests.load(Ordering::SeqCst), 1);
    server.abort();
}

use super::*;
