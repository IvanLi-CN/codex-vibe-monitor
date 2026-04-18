#[tokio::test]
async fn current_quota_route_failure_survives_informational_account_updates() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Quota exhausted after edit").await;

    record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            Some("sticky-quota-after-edit"),
            StatusCode::TOO_MANY_REQUESTS,
            "oauth_upstream_rejected_request: pool upstream responded with 429: The usage limit has been reached",
            Some("invk_quota_after_edit"),
        )
        .await
        .expect("record wrapped 429 route failure before edit");

    record_account_update_action(
        &pool,
        account_id,
        "account settings were updated after the quota-exhausted failure",
    )
    .await
    .expect("record account update action");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load updated row")
        .expect("updated row exists");
    let summary = build_summary_from_row(
        &row,
        None,
        row.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );

    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(
        summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(
        summary.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_ACCOUNT_UPDATED)
    );
    assert_eq!(
        row.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
}

async fn insert_limit_sample(
    pool: &SqlitePool,
    account_id: i64,
    captured_at: &str,
    plan_type: Option<&str>,
) {
    sqlx::query(
        r#"
            INSERT INTO pool_upstream_account_limit_samples (
                account_id, captured_at, limit_id, limit_name, plan_type,
                primary_used_percent, primary_window_minutes, primary_resets_at,
                secondary_used_percent, secondary_window_minutes, secondary_resets_at,
                credits_has_credits, credits_unlimited, credits_balance
            ) VALUES (
                ?1, ?2, NULL, NULL, ?3,
                NULL, NULL, NULL,
                NULL, NULL, NULL,
                NULL, NULL, NULL
            )
            "#,
    )
    .bind(account_id)
    .bind(captured_at)
    .bind(plan_type)
    .execute(pool)
    .await
    .expect("insert limit sample");
}

async fn insert_limit_sample_with_usage(
    pool: &SqlitePool,
    account_id: i64,
    captured_at: &str,
    primary_used_percent: Option<f64>,
    secondary_used_percent: Option<f64>,
) {
    sqlx::query(
        r#"
            INSERT INTO pool_upstream_account_limit_samples (
                account_id, captured_at, limit_id, limit_name, plan_type,
                primary_used_percent, primary_window_minutes, primary_resets_at,
                secondary_used_percent, secondary_window_minutes, secondary_resets_at,
                credits_has_credits, credits_unlimited, credits_balance
            ) VALUES (
                ?1, ?2, NULL, NULL, 'team',
                ?3, 300, NULL,
                ?4, 10080, NULL,
                NULL, NULL, NULL
            )
            "#,
    )
    .bind(account_id)
    .bind(captured_at)
    .bind(primary_used_percent)
    .bind(secondary_used_percent)
    .execute(pool)
    .await
    .expect("insert limit sample with usage");
}

async fn seed_route_cooldown(
    pool: &SqlitePool,
    account_id: i64,
    failure_kind: &str,
    cooldown_secs: i64,
) {
    let now = Utc::now();
    let now_iso = format_utc_iso(now);
    let cooldown_until = format_utc_iso(now + ChronoDuration::seconds(cooldown_secs));
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET status = ?2,
                last_error = ?3,
                last_error_at = ?4,
                last_route_failure_at = ?4,
                last_route_failure_kind = ?5,
                cooldown_until = ?6,
                consecutive_route_failures = 1,
                temporary_route_failure_streak_started_at = NULL,
                updated_at = ?4
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind("seed route cooldown")
    .bind(&now_iso)
    .bind(failure_kind)
    .bind(&cooldown_until)
    .execute(pool)
    .await
    .expect("seed route cooldown");
}

#[test]
fn pool_blocked_failure_kinds_are_not_temporary_route_failures() {
    assert!(!route_failure_kind_is_temporary(Some(
        PROXY_FAILURE_POOL_ROUTING_BLOCKED,
    )));
    assert!(!route_failure_kind_is_temporary(Some(
        PROXY_FAILURE_POOL_ASSIGNED_ACCOUNT_BLOCKED,
    )));
}

async fn seed_hard_unavailable_route_failure(
    pool: &SqlitePool,
    account_id: i64,
    status: &str,
    failure_kind: &str,
    reason_code: &str,
    http_status: Option<i64>,
) {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET status = ?2,
                last_error = ?3,
                last_error_at = ?4,
                last_route_failure_at = ?4,
                last_route_failure_kind = ?5,
                cooldown_until = NULL,
                consecutive_route_failures = 1,
                temporary_route_failure_streak_started_at = NULL,
                last_action = ?6,
                last_action_source = ?7,
                last_action_reason_code = ?8,
                last_action_reason_message = ?3,
                last_action_http_status = ?9,
                last_action_at = ?4,
                updated_at = ?4
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind(status)
    .bind("seed hard unavailable")
    .bind(&now_iso)
    .bind(failure_kind)
    .bind(UPSTREAM_ACCOUNT_ACTION_ROUTE_HARD_UNAVAILABLE)
    .bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL)
    .bind(reason_code)
    .bind(http_status)
    .execute(pool)
    .await
    .expect("seed hard unavailable");
}

#[tokio::test]
async fn record_pool_route_success_does_not_clear_newer_route_failure_state() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Stale Success Guard").await;
    seed_hard_unavailable_route_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    record_pool_route_success(
        &pool,
        account_id,
        Utc::now() - ChronoDuration::minutes(5),
        Some("sticky-stale-success"),
        Some("invk_stale_success"),
    )
    .await
    .expect("record stale route success");

    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after stale success")
        .expect("row exists after stale success");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_ROUTE_HARD_UNAVAILABLE)
    );
    assert!(
        load_sticky_route(&pool, "sticky-stale-success")
            .await
            .expect("load sticky route after stale success")
            .is_none()
    );
}

#[tokio::test]
async fn mark_account_sync_success_preserves_route_cooldown_state() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Cooldown OAuth").await;
    seed_route_cooldown(
        &pool,
        account_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        300,
    )
    .await;

    let before = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row before sync")
        .expect("row exists before sync");
    mark_account_sync_success(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MANUAL,
        SyncSuccessRouteState::PreserveFailureState,
    )
    .await
    .expect("mark sync success");
    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after sync")
        .expect("row exists after sync");

    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_synced_at.is_some());
    assert!(after.last_successful_sync_at.is_some());
    assert_eq!(after.last_route_failure_at, before.last_route_failure_at);
    assert_eq!(
        after.last_route_failure_kind,
        before.last_route_failure_kind
    );
    assert_eq!(after.cooldown_until, before.cooldown_until);
    assert_eq!(
        after.consecutive_route_failures,
        before.consecutive_route_failures
    );
}

#[tokio::test]
async fn mark_account_sync_success_clears_hard_unavailable_state_when_requested() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Recovered OAuth").await;
    seed_hard_unavailable_route_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    mark_account_sync_success(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MANUAL,
        SyncSuccessRouteState::ClearFailureState,
    )
    .await
    .expect("mark sync success");

    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after sync success")
        .expect("row exists after sync success");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_error.is_none());
    assert!(after.last_route_failure_kind.is_none());
    assert!(after.cooldown_until.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED)
    );
}

#[tokio::test]
async fn sync_api_key_account_preserves_route_cooldown_state() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Cooldown API Key").await;
    seed_route_cooldown(
        &pool,
        account_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        300,
    )
    .await;
    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load api key row")
        .expect("api key row exists");

    sync_api_key_account(&pool, &row, SyncCause::Manual)
        .await
        .expect("sync api key account");
    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after api key sync")
        .expect("row exists after api key sync");

    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    );
    assert!(after.cooldown_until.is_some());
    assert_eq!(after.consecutive_route_failures, 1);
}

#[tokio::test]
async fn sync_api_key_account_keeps_hard_unavailable_accounts_blocked() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Blocked API Key").await;
    seed_hard_unavailable_route_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load api key row")
        .expect("api key row exists");

    sync_api_key_account(&pool, &row, SyncCause::Manual)
        .await
        .expect("sync api key account");
    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after api key sync")
        .expect("row exists after api key sync");

    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert!(after.last_synced_at.is_some());
    assert!(after.last_successful_sync_at.is_none());
    assert_eq!(after.last_error.as_deref(), Some("seed hard unavailable"));
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_RECOVERY_BLOCKED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED)
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
}

#[tokio::test]
async fn sync_scope_reuses_live_reserved_node_for_same_account_before_shared_group_probe() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Reserved OAuth",
        "reserved@example.com",
        "org_reserved",
        "user_reserved",
    )
    .await;

    set_test_account_group_name(&state.pool, account_id, Some("node-shunt-sync-reserved")).await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-sync-reserved",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: test_required_group_bound_proxy_keys(),
            node_shunt_enabled: true,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save reserved node shunt sync metadata");
    drop(conn);

    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned")
        .insert(
            "test-node-shunt-sync-reservation".to_string(),
            PoolRoutingReservation {
                account_id,
                proxy_key: Some(FORWARD_PROXY_DIRECT_KEY.to_string()),
                created_at: Instant::now(),
            },
        );

    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load reserved account")
        .expect("reserved account exists");
    let scope = resolve_account_forward_proxy_scope_for_sync(state.as_ref(), &row, None)
        .await
        .expect("sync scope should reuse same-account live reservation");

    let ForwardProxyRouteScope::PinnedProxyKey(proxy_key) = scope else {
        panic!("expected sync scope to pin the live reserved node");
    };
    assert_eq!(proxy_key, FORWARD_PROXY_DIRECT_KEY);
}

#[tokio::test]
async fn oauth_sync_refresh_due_reuses_sync_only_scope_for_token_refresh() {
    let (proxy_url, usage_requests, token_requests, server) =
        spawn_proxy_only_oauth_sync_server().await;
    let state = test_app_state_with_usage_and_oauth_base(
        "http://unreachable.invalid/backend-api",
        "http://unreachable.invalid",
    )
    .await;
    let secondary_proxy_key = {
        let mut manager = state.forward_proxy.lock().await;
        let mut settings = ForwardProxySettings::default();
        settings.proxy_urls = vec![proxy_url];
        manager.apply_settings(settings);
        manager.bound_group_runtime.insert(
            "node-shunt-refresh".to_string(),
            crate::forward_proxy::BoundForwardProxyGroupState {
                current_binding_key: Some(FORWARD_PROXY_DIRECT_KEY.to_string()),
                consecutive_network_failures: 0,
            },
        );
        manager
            .binding_nodes()
            .into_iter()
            .find(|node| node.key != FORWARD_PROXY_DIRECT_KEY)
            .map(|node| node.key)
            .expect("secondary proxy binding key")
    };
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Refresh Due Scoped OAuth",
        "proxy-refresh@example.com",
        "org_proxy_refresh",
        "user_proxy_refresh",
    )
    .await;

    set_test_account_group_name(&state.pool, account_id, Some("node-shunt-refresh")).await;
    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-refresh",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: vec![
                FORWARD_PROXY_DIRECT_KEY.to_string(),
                secondary_proxy_key.clone(),
            ],
            node_shunt_enabled: true,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save refresh-due node shunt metadata");
    drop(conn);

    state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned")
        .insert(
            "test-node-shunt-refresh-reservation".to_string(),
            PoolRoutingReservation {
                account_id,
                proxy_key: Some(secondary_proxy_key),
                created_at: Instant::now(),
            },
        );
    set_test_account_token_expires_at(
        &state.pool,
        account_id,
        &format_utc_iso(Utc::now() - ChronoDuration::minutes(5)),
    )
    .await;

    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load refresh-due account")
        .expect("refresh-due account exists");
    sync_oauth_account(state.as_ref(), &row, SyncCause::Manual)
        .await
        .expect("refresh-due sync should reuse the sync-only scoped node for refresh");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load refresh-due account after sync")
        .expect("refresh-due account still exists");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_error.is_none());
    assert!(after.last_route_failure_kind.is_none());
    assert!(after.last_successful_sync_at.is_some());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED)
    );
    assert_eq!(token_requests.load(Ordering::SeqCst), 1);
    assert_eq!(usage_requests.load(Ordering::SeqCst), 1);

    let decrypted = decrypt_credentials(
        crypto_key,
        after
            .encrypted_credentials
            .as_deref()
            .expect("encrypted oauth credentials"),
    )
    .expect("decrypt refreshed credentials");
    let StoredCredentials::Oauth(credentials) = decrypted else {
        panic!("unexpected credential kind after refresh-due sync")
    };
    assert_eq!(credentials.access_token, "proxy-refreshed-access-token");
    assert_eq!(credentials.refresh_token, "proxy-refreshed-refresh-token");

    server.abort();
}

#[tokio::test]
async fn sync_scope_falls_back_to_shared_bound_group_when_exclusive_slot_is_full() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let occupying_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Occupying OAuth",
        "occupying@example.com",
        "org_occupying",
        "user_occupying",
    )
    .await;
    let queued_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Queued OAuth",
        "queued@example.com",
        "org_queued",
        "user_queued",
    )
    .await;

    set_test_account_group_name(&state.pool, occupying_account_id, Some("node-shunt-sync")).await;
    set_test_account_group_name(&state.pool, queued_account_id, Some("node-shunt-sync")).await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-sync",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: test_required_group_bound_proxy_keys(),
            node_shunt_enabled: true,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save node shunt sync metadata");
    drop(conn);

    seed_hard_unavailable_route_failure(
        &state.pool,
        queued_account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    let assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
        .await
        .expect("build node shunt assignments");
    assert_eq!(
        assignments
            .account_proxy_keys
            .get(&occupying_account_id)
            .map(String::as_str),
        Some(FORWARD_PROXY_DIRECT_KEY),
    );
    assert!(
        !assignments
            .account_proxy_keys
            .contains_key(&queued_account_id),
        "queued account should remain unassigned when the only slot is occupied",
    );

    let row = load_upstream_account_row(&state.pool, queued_account_id)
        .await
        .expect("load queued account")
        .expect("queued account exists");
    let scope = resolve_account_forward_proxy_scope_for_sync(state.as_ref(), &row, None)
        .await
        .expect("sync scope should fall back to shared bound-group probe");

    let ForwardProxyRouteScope::BoundGroup {
        group_name,
        bound_proxy_keys,
    } = scope
    else {
        panic!("expected sync scope to probe the bound group without claiming an exclusive slot");
    };
    assert_eq!(group_name, "node-shunt-sync");
    assert_eq!(bound_proxy_keys, test_required_group_bound_proxy_keys());
}

#[tokio::test]
async fn manual_sync_allows_group_node_shunt_unassigned_account_to_probe_bound_node() {
    let (base_url, server) = spawn_usage_snapshot_server(
        StatusCode::OK,
        json!({
            "planType": "team",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 42,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                }
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let occupying_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Occupying OAuth",
        "occupying@example.com",
        "org_occupying",
        "user_occupying",
    )
    .await;
    let queued_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Queued OAuth",
        "queued@example.com",
        "org_queued",
        "user_queued",
    )
    .await;

    set_test_account_group_name(&state.pool, occupying_account_id, Some("node-shunt-sync")).await;
    set_test_account_group_name(&state.pool, queued_account_id, Some("node-shunt-sync")).await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-sync",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: test_required_group_bound_proxy_keys(),
            node_shunt_enabled: true,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save node shunt sync metadata");
    drop(conn);

    seed_hard_unavailable_route_failure(
        &state.pool,
        queued_account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    let assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
        .await
        .expect("build node shunt assignments");
    assert_eq!(
        assignments
            .account_proxy_keys
            .get(&occupying_account_id)
            .map(String::as_str),
        Some(FORWARD_PROXY_DIRECT_KEY),
    );
    assert!(
        !assignments
            .account_proxy_keys
            .contains_key(&queued_account_id),
        "queued account should remain unassigned when the only slot is occupied",
    );

    let detail = state
        .upstream_accounts
        .account_ops
        .run_manual_sync(state.clone(), queued_account_id)
        .await
        .expect("queued account manual sync should fall back to the shared bound node");

    let after = load_upstream_account_row(&state.pool, queued_account_id)
        .await
        .expect("load queued account after sync")
        .expect("queued account still exists");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_error.is_none());
    assert!(after.last_route_failure_kind.is_none());
    assert!(after.last_successful_sync_at.is_some());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED)
    );
    assert_eq!(detail.summary.id, queued_account_id);
    assert_eq!(
        detail.summary.routing_block_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_GROUP_NODE_SHUNT_UNASSIGNED),
    );
    assert_eq!(
        detail.summary.routing_block_reason_message.as_deref(),
        Some(group_node_shunt_unassigned_error_message()),
    );

    server.abort();
}

#[tokio::test]
async fn maintenance_sync_allows_group_node_shunt_unassigned_account_to_probe_bound_node() {
    let (base_url, server) = spawn_usage_snapshot_server(
        StatusCode::OK,
        json!({
            "planType": "team",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 42,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                }
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let occupying_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Occupying Maintenance OAuth",
        "occupying-maintenance@example.com",
        "org_occupying_maintenance",
        "user_occupying_maintenance",
    )
    .await;
    let queued_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Queued Maintenance OAuth",
        "queued-maintenance@example.com",
        "org_queued_maintenance",
        "user_queued_maintenance",
    )
    .await;

    set_test_account_group_name(&state.pool, occupying_account_id, Some("node-shunt-maint")).await;
    set_test_account_group_name(&state.pool, queued_account_id, Some("node-shunt-maint")).await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-maint",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: test_required_group_bound_proxy_keys(),
            node_shunt_enabled: true,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save node shunt maintenance metadata");
    drop(conn);

    seed_hard_unavailable_route_failure(
        &state.pool,
        queued_account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    let outcome = state
        .upstream_accounts
        .account_ops
        .run_maintenance_sync(state.clone(), queued_account_id)
        .await
        .expect("maintenance sync should execute via shared bound-node probe");
    assert!(matches!(outcome, MaintenanceDispatchOutcome::Executed));

    let after = load_upstream_account_row(&state.pool, queued_account_id)
        .await
        .expect("load queued maintenance account after sync")
        .expect("queued maintenance account still exists");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_error.is_none());
    assert!(after.last_route_failure_kind.is_none());
    assert!(after.last_successful_sync_at.is_some());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED)
    );
    let detail = load_upstream_account_detail_with_actual_usage(state.as_ref(), queued_account_id)
        .await
        .expect("load queued maintenance detail")
        .expect("queued maintenance detail exists");
    assert_eq!(
        detail.summary.routing_block_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_GROUP_NODE_SHUNT_UNASSIGNED),
    );

    server.abort();
}

#[tokio::test]
async fn bulk_sync_allows_group_node_shunt_unassigned_account_to_probe_bound_node() {
    let (base_url, server) = spawn_usage_snapshot_server(
        StatusCode::OK,
        json!({
            "planType": "team",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 42,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                }
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let occupying_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Occupying Bulk OAuth",
        "occupying-bulk@example.com",
        "org_occupying_bulk",
        "user_occupying_bulk",
    )
    .await;
    let queued_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Queued Bulk OAuth",
        "queued-bulk@example.com",
        "org_queued_bulk",
        "user_queued_bulk",
    )
    .await;

    set_test_account_group_name(&state.pool, occupying_account_id, Some("node-shunt-bulk")).await;
    set_test_account_group_name(&state.pool, queued_account_id, Some("node-shunt-bulk")).await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-bulk",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: test_required_group_bound_proxy_keys(),
            node_shunt_enabled: true,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save node shunt bulk metadata");
    drop(conn);

    seed_hard_unavailable_route_failure(
        &state.pool,
        queued_account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    let response = create_bulk_upstream_account_sync_job(
        State(state.clone()),
        HeaderMap::new(),
        Json(BulkUpstreamAccountSyncJobRequest {
            account_ids: vec![queued_account_id],
        }),
    )
    .await
    .expect("create bulk sync job")
    .0;
    let job = state
        .upstream_accounts
        .get_bulk_sync_job(&response.job_id)
        .await
        .expect("bulk sync job exists");
    let mut terminal = None;
    for _ in 0..100 {
        terminal = job.terminal_event.lock().await.clone();
        if terminal.is_some() {
            break;
        }
        sleep(Duration::from_millis(10)).await;
    }
    let Some(BulkUpstreamAccountSyncTerminalEvent::Completed(payload)) = terminal else {
        panic!("bulk sync job should complete successfully");
    };
    assert_eq!(payload.counts.total, 1);
    assert_eq!(payload.counts.completed, 1);
    assert_eq!(payload.counts.failed, 0);
    assert_eq!(payload.snapshot.rows.len(), 1);
    assert_eq!(
        payload.snapshot.rows[0].status,
        BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_SUCCEEDED
    );
    assert_eq!(payload.snapshot.rows[0].account_id, queued_account_id);

    let after = load_upstream_account_row(&state.pool, queued_account_id)
        .await
        .expect("load queued bulk account after sync")
        .expect("queued bulk account still exists");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_successful_sync_at.is_some());

    server.abort();
}

#[tokio::test]
async fn detail_preserves_group_node_shunt_unassigned_routing_block_reason() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let occupying_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Occupying OAuth",
        "occupying@example.com",
        "org_occupying",
        "user_occupying",
    )
    .await;
    let queued_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Queued OAuth",
        "queued@example.com",
        "org_queued",
        "user_queued",
    )
    .await;

    set_test_account_group_name(&state.pool, occupying_account_id, Some("node-shunt-sync")).await;
    set_test_account_group_name(&state.pool, queued_account_id, Some("node-shunt-sync")).await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-sync",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: test_required_group_bound_proxy_keys(),
            node_shunt_enabled: true,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save node shunt sync metadata");
    drop(conn);

    let detail = load_upstream_account_detail_with_actual_usage(state.as_ref(), queued_account_id)
        .await
        .expect("load queued account detail")
        .expect("queued account detail exists");
    assert_eq!(detail.summary.id, queued_account_id);
    assert_eq!(
        detail.summary.routing_block_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_GROUP_NODE_SHUNT_UNASSIGNED),
    );
    assert_eq!(
        detail.summary.routing_block_reason_message.as_deref(),
        Some(group_node_shunt_unassigned_error_message()),
    );
}

#[tokio::test]
async fn list_upstream_accounts_applies_node_shunt_idle_rewrite_before_filters() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let occupying_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Occupying Filtered OAuth",
        "occupying-filtered@example.com",
        "org_occupying_filtered",
        "user_occupying_filtered",
    )
    .await;
    let queued_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Queued Filtered OAuth",
        "queued-filtered@example.com",
        "org_queued_filtered",
        "user_queued_filtered",
    )
    .await;

    set_test_account_group_name(&state.pool, occupying_account_id, Some("node-shunt-filter")).await;
    set_test_account_group_name(&state.pool, queued_account_id, Some("node-shunt-filter")).await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-filter",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: test_required_group_bound_proxy_keys(),
            node_shunt_enabled: true,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save node shunt filter metadata");
    drop(conn);

    let occupying_selected_at = format_utc_iso(Utc::now() - ChronoDuration::minutes(3));
    let queued_selected_at = format_utc_iso(Utc::now() - ChronoDuration::minutes(1));
    for (account_id, selected_at) in [
        (occupying_account_id, occupying_selected_at),
        (queued_account_id, queued_selected_at),
    ] {
        sqlx::query(
            r#"
                UPDATE pool_upstream_accounts
                SET last_selected_at = ?2,
                    updated_at = ?2
                WHERE id = ?1
                "#,
        )
        .bind(account_id)
        .bind(selected_at)
        .execute(&state.pool)
        .await
        .expect("seed last_selected_at");
    }

    let assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
        .await
        .expect("build node shunt assignments");
    assert_eq!(
        assignments
            .account_proxy_keys
            .get(&occupying_account_id)
            .map(String::as_str),
        Some(FORWARD_PROXY_DIRECT_KEY),
    );
    assert!(
        !assignments
            .account_proxy_keys
            .contains_key(&queued_account_id),
        "queued account should be unassigned before list filtering",
    );

    let mut all_items = load_upstream_account_summaries_for_query(
        &state.pool,
        &ListUpstreamAccountsQuery::default(),
    )
    .await
    .expect("load upstream account summaries");
    enrich_node_shunt_routing_block_reasons(state.as_ref(), &mut all_items)
        .await
        .expect("enrich node shunt routing block reasons");

    let idle_filters = normalize_upstream_account_list_filters(&ListUpstreamAccountsQuery {
        work_status: vec![UPSTREAM_ACCOUNT_WORK_STATUS_IDLE.to_string()],
        ..ListUpstreamAccountsQuery::default()
    });
    let idle_items = filter_upstream_account_summaries(all_items.clone(), &idle_filters);
    let idle_metrics = build_upstream_account_list_metrics(&idle_items);

    assert_eq!(idle_items.len(), 1);
    assert_eq!(idle_metrics.total, 1);
    assert_eq!(idle_items[0].id, queued_account_id);
    assert_eq!(idle_items[0].work_status, UPSTREAM_ACCOUNT_WORK_STATUS_IDLE);
    assert_eq!(
        idle_items[0].routing_block_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_GROUP_NODE_SHUNT_UNASSIGNED),
    );

    let working_filters = normalize_upstream_account_list_filters(&ListUpstreamAccountsQuery {
        work_status: vec![UPSTREAM_ACCOUNT_WORK_STATUS_WORKING.to_string()],
        ..ListUpstreamAccountsQuery::default()
    });
    let working_items = filter_upstream_account_summaries(all_items, &working_filters);
    assert_eq!(working_items.len(), 1);
    assert_eq!(working_items[0].id, occupying_account_id);
    assert!(
        working_items
            .iter()
            .all(|item| item.id != queued_account_id),
        "queued account should not remain in working results after node shunt rewrite",
    )
}

#[tokio::test]
async fn sync_api_key_account_clears_stale_manual_recovery_marker_on_active_rows() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Active API Key With Stale Marker").await;
    seed_hard_unavailable_route_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    mark_account_sync_success(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
        SyncSuccessRouteState::PreserveFailureState,
    )
    .await
    .expect("mark legacy sync success");
    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load api key row")
        .expect("api key row exists");
    assert_eq!(row.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(
        row.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );

    sync_api_key_account(&pool, &row, SyncCause::Maintenance)
        .await
        .expect("sync api key account");
    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after api key sync")
        .expect("row exists after api key sync");

    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_route_failure_kind.is_none());
    assert!(after.cooldown_until.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED)
    );
}

#[tokio::test]
async fn updating_api_key_reactivates_manually_recoverable_account() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Recoverable API Key").await;
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            account_id,
            UpdateUpstreamAccountRequest {
                display_name: None,
                group_name: None,
                group_bound_proxy_keys: None,
                group_node_shunt_enabled: None,
                note: None,
                group_note: None,
                concurrency_limit: None,
                upstream_base_url: OptionalField::Missing,
                enabled: None,
                is_mother: None,
                api_key: Some("sk-live-new".to_string()),
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: None,
            },
        )
        .await
        .expect("update api key account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load row after api key update")
        .expect("row exists after api key update");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_error.is_none());
    assert!(after.last_route_failure_kind.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_ACCOUNT_UPDATED)
    );
}

#[tokio::test]
async fn oauth_sync_keeps_quota_exhausted_accounts_blocked_until_snapshot_recovers() {
    let (base_url, server) = spawn_usage_snapshot_server(
        StatusCode::OK,
        json!({
            "planType": "team",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 100,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                }
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Exhausted OAuth",
        "exhausted@example.com",
        "org_exhausted",
        "user_exhausted",
    )
    .await;
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");

    sync_oauth_account(&state, &row, SyncCause::Manual)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after sync")
        .expect("oauth row exists after sync");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert!(after.last_synced_at.is_some());
    assert!(after.last_successful_sync_at.is_none());
    assert_eq!(after.last_error.as_deref(), Some("seed hard unavailable"));
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_RECOVERY_BLOCKED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    server.abort();
}

#[tokio::test]
async fn oauth_sync_ignores_stale_input_row_after_newer_quota_hard_stop() {
    let (base_url, server) = spawn_usage_snapshot_server(
        StatusCode::OK,
        json!({
            "planType": "team",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 100,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                }
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Stale OAuth Input Row",
        "stale-input@example.com",
        "org_stale_input",
        "user_stale_input",
    )
    .await;
    let stale_row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    sync_oauth_account(&state, &stale_row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after stale sync")
        .expect("oauth row exists after stale sync");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_RECOVERY_BLOCKED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    server.abort();
}

#[tokio::test]
async fn oauth_sync_demotes_active_stale_quota_marker_when_snapshot_is_still_exhausted() {
    let (base_url, server) = spawn_usage_snapshot_server(
        StatusCode::OK,
        json!({
            "planType": "team",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 100,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                }
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Legacy Active Exhausted OAuth",
        "legacy-exhausted@example.com",
        "org_legacy_exhausted",
        "user_legacy_exhausted",
    )
    .await;
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    mark_account_sync_success(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
        SyncSuccessRouteState::PreserveFailureState,
    )
    .await
    .expect("mark legacy sync success");
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");
    assert_eq!(row.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(
        row.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );

    sync_oauth_account(&state, &row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after sync")
        .expect("oauth row exists after sync");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_RECOVERY_BLOCKED)
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    server.abort();
}

#[tokio::test]
async fn oauth_sync_reactivates_quota_exhausted_account_once_snapshot_recovers() {
    let (base_url, server) = spawn_usage_snapshot_server(
        StatusCode::OK,
        json!({
            "planType": "team",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 42,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                }
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Recovered OAuth Sync",
        "recovered@example.com",
        "org_recovered",
        "user_recovered",
    )
    .await;
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");

    sync_oauth_account(&state, &row, SyncCause::Manual)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after recovery")
        .expect("oauth row exists after recovery");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_error.is_none());
    assert!(after.last_route_failure_kind.is_none());
    assert!(after.last_successful_sync_at.is_some());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED)
    );
    server.abort();
}

#[tokio::test]
async fn oauth_sync_retry_after_refresh_settles_to_needs_reauth_without_stale_syncing() {
    let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
            spawn_sequenced_oauth_sync_server(
                vec![
                    (
                        StatusCode::UNAUTHORIZED,
                        json!({
                            "error": {
                                "message": "Session cookie expired during usage snapshot"
                            }
                        }),
                    ),
                    (
                        StatusCode::FORBIDDEN,
                        json!({
                            "error": {
                                "message": "Authentication token has been invalidated, please sign in again"
                            }
                        }),
                    ),
                ],
                json!({
                    "access_token": "refreshed-access-token",
                    "refresh_token": "refresh-token-rotated",
                    "id_token": test_id_token(
                        "reauth-required@example.com",
                        Some("org_retry_reauth"),
                        Some("user_retry_reauth"),
                        Some("team"),
                    ),
                    "token_type": "Bearer",
                    "expires_in": 3600
                }),
            )
            .await;
    let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Retry Needs Reauth OAuth",
        "reauth-required@example.com",
        "org_retry_reauth",
        "user_retry_reauth",
    )
    .await;
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");

    sync_oauth_account(&state, &row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after retry failure")
        .expect("oauth row exists after retry failure");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
    assert!(after.last_synced_at.is_some());
    assert!(after.last_successful_sync_at.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED)
    );
    assert_eq!(after.last_action_http_status, Some(403));
    assert_eq!(
        after.last_error.as_deref(),
        Some(
            "usage endpoint returned 403 Forbidden: Authentication token has been invalidated, please sign in again"
        )
    );
    assert!(after.last_action_at.is_some());

    let decrypted = decrypt_credentials(
        crypto_key,
        after
            .encrypted_credentials
            .as_deref()
            .expect("encrypted oauth credentials"),
    )
    .expect("decrypt refreshed credentials");
    let StoredCredentials::Oauth(credentials) = decrypted else {
        panic!("unexpected credential kind after refresh")
    };
    assert_eq!(credentials.access_token, "refreshed-access-token");
    assert_eq!(credentials.refresh_token, "refresh-token-rotated");

    let summary = build_summary_from_row(
        &after,
        None,
        after.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
    assert_eq!(
        summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
    );
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);

    let detail = load_upstream_account_detail(&state.pool, account_id)
        .await
        .expect("load detail export")
        .expect("detail export exists");
    assert_eq!(
        detail.summary.display_status,
        UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH
    );
    assert_eq!(
        detail.summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
    );
    assert_eq!(detail.summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
    assert_eq!(
        detail
            .recent_actions
            .first()
            .map(|event| event.action.as_str()),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        detail
            .recent_actions
            .first()
            .and_then(|event| event.reason_code.as_deref()),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED)
    );
    assert_eq!(usage_requests.load(Ordering::SeqCst), 2);
    assert_eq!(token_requests.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
async fn oauth_sync_retry_after_refresh_records_non_auth_terminal_failure_without_stale_syncing() {
    let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
        spawn_sequenced_oauth_sync_server(
            vec![
                (
                    StatusCode::UNAUTHORIZED,
                    json!({
                        "error": {
                            "message": "Session cookie expired during usage snapshot"
                        }
                    }),
                ),
                (
                    StatusCode::BAD_GATEWAY,
                    json!({
                        "error": {
                            "message": "gateway temporarily unavailable"
                        }
                    }),
                ),
            ],
            json!({
                "access_token": "refreshed-temporary-token",
                "refresh_token": "refresh-token-rotated",
                "id_token": test_id_token(
                    "transport-failure@example.com",
                    Some("org_retry_gateway"),
                    Some("user_retry_gateway"),
                    Some("team"),
                ),
                "token_type": "Bearer",
                "expires_in": 3600
            }),
        )
        .await;
    let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Retry Gateway Failure OAuth",
        "transport-failure@example.com",
        "org_retry_gateway",
        "user_retry_gateway",
    )
    .await;
    seed_route_cooldown(
        &state.pool,
        account_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        300,
    )
    .await;
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");

    sync_oauth_account(&state, &row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after gateway failure")
        .expect("oauth row exists after gateway failure");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_synced_at.is_some());
    assert!(after.last_successful_sync_at.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some("upstream_http_5xx")
    );
    assert_eq!(after.last_action_http_status, Some(502));
    assert_eq!(
        after.last_error.as_deref(),
        Some("usage endpoint returned 502 Bad Gateway: gateway temporarily unavailable")
    );
    assert!(after.last_action_at.is_some());
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    );
    assert!(after.cooldown_until.is_some());
    assert_eq!(after.consecutive_route_failures, 1);

    let summary = build_summary_from_row(
        &after,
        None,
        after.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.work_status, UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED);
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);

    let detail = load_upstream_account_detail(&state.pool, account_id)
        .await
        .expect("load detail export")
        .expect("detail export exists");
    assert_eq!(
        detail.summary.display_status,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE
    );
    assert_eq!(
        detail.summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED
    );
    assert_eq!(detail.summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
    assert_eq!(
        detail
            .recent_actions
            .first()
            .map(|event| event.action.as_str()),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        detail
            .recent_actions
            .first()
            .and_then(|event| event.http_status),
        Some(502)
    );
    assert_eq!(usage_requests.load(Ordering::SeqCst), 2);
    assert_eq!(token_requests.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
async fn oauth_sync_retry_after_refresh_preserves_quota_marker_from_current_db_state() {
    let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
        spawn_sequenced_oauth_sync_server(
            vec![
                (
                    StatusCode::UNAUTHORIZED,
                    json!({
                        "error": {
                            "message": "Session cookie expired during usage snapshot"
                        }
                    }),
                ),
                (
                    StatusCode::BAD_GATEWAY,
                    json!({
                        "error": {
                            "message": "gateway temporarily unavailable"
                        }
                    }),
                ),
            ],
            json!({
                "access_token": "refreshed-quota-preserving-token",
                "refresh_token": "refresh-token-rotated",
                "id_token": test_id_token(
                    "retry-quota-preserve@example.com",
                    Some("org_retry_quota_preserve"),
                    Some("user_retry_quota_preserve"),
                    Some("team"),
                ),
                "token_type": "Bearer",
                "expires_in": 3600
            }),
        )
        .await;
    let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Retry Gateway Quota Preserve OAuth",
        "retry-quota-preserve@example.com",
        "org_retry_quota_preserve",
        "user_retry_quota_preserve",
    )
    .await;
    let stale_row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    sync_oauth_account(&state, &stale_row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after gateway failure")
        .expect("oauth row exists after gateway failure");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_synced_at.is_some());
    assert!(after.last_successful_sync_at.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some("upstream_http_5xx")
    );
    assert_eq!(after.last_action_http_status, Some(502));
    assert_eq!(
        after.last_error.as_deref(),
        Some("usage endpoint returned 502 Bad Gateway: gateway temporarily unavailable")
    );
    assert!(after.last_action_at.is_some());
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    assert_eq!(after.last_route_failure_at, after.last_error_at);

    let summary = build_summary_from_row(
        &after,
        None,
        after.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(
        summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);

    let detail = load_upstream_account_detail(&state.pool, account_id)
        .await
        .expect("load detail export")
        .expect("detail export exists");
    assert_eq!(
        detail.summary.display_status,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE
    );
    assert_eq!(
        detail.summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(detail.summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);

    assert_eq!(usage_requests.load(Ordering::SeqCst), 2);
    assert_eq!(token_requests.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
async fn oauth_sync_refresh_failure_preserves_quota_marker_from_current_db_state() {
    let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
        spawn_sequenced_oauth_sync_server(
            vec![(
                StatusCode::UNAUTHORIZED,
                json!({
                    "error": {
                        "message": "Session cookie expired during usage snapshot"
                    }
                }),
            )],
            json!({
                "unexpected": "shape"
            }),
        )
        .await;
    let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Refresh Failure Quota Preserve OAuth",
        "refresh-quota-preserve@example.com",
        "org_refresh_quota_preserve",
        "user_refresh_quota_preserve",
    )
    .await;
    let stale_row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    sync_oauth_account(&state, &stale_row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after refresh failure")
        .expect("oauth row exists after refresh failure");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR)
    );
    assert_eq!(after.last_action_http_status, None);
    assert!(
        after
            .last_error
            .as_deref()
            .is_some_and(|message| message.contains("failed to decode OAuth token response"))
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    assert_eq!(after.last_route_failure_at, after.last_error_at);

    let summary = build_summary_from_row(
        &after,
        None,
        after.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(
        summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);

    let detail = load_upstream_account_detail(&state.pool, account_id)
        .await
        .expect("load detail export")
        .expect("detail export exists");
    assert_eq!(
        detail.summary.display_status,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE
    );
    assert_eq!(
        detail.summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(detail.summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);

    assert_eq!(usage_requests.load(Ordering::SeqCst), 1);
    assert_eq!(token_requests.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
async fn oauth_sync_direct_fetch_failure_preserves_quota_marker_from_current_db_state() {
    let (usage_base_url, server) = spawn_usage_snapshot_server(
        StatusCode::BAD_GATEWAY,
        json!({
            "error": {
                "message": "gateway temporarily unavailable"
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&usage_base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Direct Failure Quota Preserve OAuth",
        "direct-quota-preserve@example.com",
        "org_direct_quota_preserve",
        "user_direct_quota_preserve",
    )
    .await;
    let stale_row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    sync_oauth_account(&state, &stale_row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after direct fetch failure")
        .expect("oauth row exists after direct fetch failure");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some("upstream_http_5xx")
    );
    assert_eq!(after.last_action_http_status, Some(502));
    assert!(
        after
            .last_error
            .as_deref()
            .is_some_and(|message| message.contains("502 Bad Gateway"))
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    assert_eq!(after.last_route_failure_at, after.last_error_at);

    let summary = build_summary_from_row(
        &after,
        None,
        after.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(
        summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);

    let detail = load_upstream_account_detail(&state.pool, account_id)
        .await
        .expect("load detail export")
        .expect("detail export exists");
    assert_eq!(
        detail.summary.display_status,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE
    );
    assert_eq!(
        detail.summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(detail.summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
    server.abort();
}

#[tokio::test]
async fn classified_sync_failure_preserves_existing_route_cooldown_across_new_error_timestamp() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Preserved Cooldown OAuth").await;
    let previous_failure_at = format_utc_iso(Utc::now() - ChronoDuration::minutes(2));
    let cooldown_until = format_utc_iso(Utc::now() + ChronoDuration::minutes(5));

    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET status = ?2,
                last_error = ?3,
                last_error_at = ?4,
                last_route_failure_at = ?4,
                last_route_failure_kind = ?5,
                cooldown_until = ?6,
                consecutive_route_failures = 1,
                last_action = ?7,
                last_action_source = ?8,
                last_action_reason_code = ?9,
                last_action_reason_message = ?3,
                last_action_http_status = ?10,
                last_action_at = ?4,
                updated_at = ?4
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind("seed preserved cooldown")
    .bind(&previous_failure_at)
    .bind(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    .bind(&cooldown_until)
    .bind(UPSTREAM_ACCOUNT_ACTION_ROUTE_COOLDOWN_STARTED)
    .bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL)
    .bind(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT)
    .bind(429)
    .execute(&pool)
    .await
    .expect("seed preserved cooldown row");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load seeded cooldown row")
        .expect("seeded cooldown row exists");
    record_classified_account_sync_failure(
        &pool,
        &row,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
        "usage endpoint returned 502 Bad Gateway: gateway temporarily unavailable",
    )
    .await
    .expect("record classified retry failure");

    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load cooldown row after retry failure")
        .expect("cooldown row after retry failure exists");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some("upstream_http_5xx")
    );
    assert_eq!(after.last_action_http_status, Some(502));
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    );
    assert_eq!(after.last_route_failure_at, after.last_error_at);
    assert_ne!(
        after.last_route_failure_at.as_deref(),
        Some(previous_failure_at.as_str())
    );

    let summary = build_summary_from_row(
        &after,
        None,
        after.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.work_status, UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED);
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
}

#[tokio::test]
async fn classified_sync_hard_unavailable_replaces_stale_quota_marker_from_current_syncing_row() {
    let pool = test_pool().await;

    for (reason_code, http_status, failure_kind, error_message) in [
        (
            "upstream_http_401",
            StatusCode::UNAUTHORIZED,
            PROXY_FAILURE_UPSTREAM_HTTP_AUTH,
            "usage endpoint returned 401 Unauthorized: Missing scopes: api.responses.write",
        ),
        (
            "upstream_http_402",
            StatusCode::PAYMENT_REQUIRED,
            PROXY_FAILURE_UPSTREAM_HTTP_402,
            "usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}",
        ),
        (
            "upstream_http_403",
            StatusCode::FORBIDDEN,
            PROXY_FAILURE_UPSTREAM_HTTP_AUTH,
            "usage endpoint returned 403 Forbidden: You have insufficient permissions for this operation.",
        ),
    ] {
        let account_id =
            insert_oauth_account(&pool, &format!("Syncing hard unavailable {reason_code}")).await;

        seed_hard_unavailable_route_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            Some(429),
        )
        .await;
        set_account_status(&pool, account_id, UPSTREAM_ACCOUNT_STATUS_SYNCING, None)
            .await
            .expect("mark row syncing");

        let current_row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load current syncing row")
            .expect("current syncing row exists");
        assert_eq!(current_row.status, UPSTREAM_ACCOUNT_STATUS_SYNCING);

        record_classified_account_sync_failure(
            &pool,
            &current_row,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            error_message,
        )
        .await
        .expect("record hard unavailable failure against syncing row");

        let after = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load syncing row after hard unavailable failure")
            .expect("syncing row after hard unavailable failure exists");
        assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
        assert_eq!(after.last_action_reason_code.as_deref(), Some(reason_code));
        assert_eq!(
            after.last_action_http_status,
            Some(http_status.as_u16() as i64)
        );
        assert_eq!(after.last_route_failure_kind.as_deref(), Some(failure_kind));
        assert_eq!(after.last_route_failure_at, after.last_error_at);
        if reason_code == "upstream_http_402" {
            let cooldown_until = after
                .cooldown_until
                .as_deref()
                .and_then(parse_rfc3339_utc)
                .expect("maintenance-triggered 402 should write explicit cooldown");
            let failed_at = after
                .last_action_at
                .as_deref()
                .and_then(parse_rfc3339_utc)
                .expect("maintenance-triggered 402 should record last_action_at");
            assert_eq!(
                cooldown_until - failed_at,
                ChronoDuration::seconds(
                    UPSTREAM_ACCOUNT_UPSTREAM_REJECTED_MAINTENANCE_COOLDOWN_SECS,
                )
            );
        } else {
            assert_eq!(after.cooldown_until, None);
        }
        assert_eq!(after.temporary_route_failure_streak_started_at, None);

        let summary = build_summary_from_row(
            &after,
            None,
            after.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );
        assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
        assert_eq!(
            summary.display_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
        );
        assert_eq!(
            summary.health_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
        );
        assert_eq!(
            summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
        );
        assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
    }
}

#[tokio::test]
async fn classified_sync_wrapped_upstream_rejected_permission_keeps_existing_cooldown_policy() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Wrapped upstream rejected cooldown").await;

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load fresh row")
        .expect("fresh row exists");
    record_classified_account_sync_failure(
        &pool,
        &row,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
        "oauth_upstream_rejected_request: pool upstream responded with 403: Forbidden",
    )
    .await
    .expect("record wrapped upstream rejected sync failure");

    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after wrapped upstream rejected sync failure")
        .expect("row after wrapped upstream rejected sync failure exists");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some("upstream_http_403")
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_AUTH)
    );
    assert_eq!(
        after.cooldown_until, None,
        "wrapped upstream auth errors should keep the old no-cooldown behavior"
    );
}

#[tokio::test]
async fn classified_sync_non_rejected_failure_clears_existing_maintenance_rejected_cooldown() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Rejected Cooldown Replaced").await;

    record_account_sync_hard_unavailable(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            "upstream_http_402",
            "usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}",
            PROXY_FAILURE_UPSTREAM_HTTP_402,
        )
        .await
        .expect("seed maintenance rejected cooldown");

    let before = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row before replacement failure")
        .expect("row exists before replacement failure");
    assert!(before.cooldown_until.is_some());

    record_classified_account_sync_failure(
            &pool,
            &before,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            "usage endpoint returned 403 Forbidden: You have insufficient permissions for this operation.",
        )
        .await
        .expect("record replacement sync failure");

    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after replacement failure")
        .expect("row exists after replacement failure");
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some("upstream_http_403")
    );
    assert_eq!(after.cooldown_until, None);
}

#[tokio::test]
async fn mark_account_sync_success_clears_explicit_maintenance_upstream_rejected_cooldown() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Rejected Cooldown Success").await;

    record_account_sync_hard_unavailable(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            "upstream_http_402",
            "usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}",
            PROXY_FAILURE_UPSTREAM_HTTP_402,
        )
        .await
        .expect("seed maintenance rejected cooldown");

    let before = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row before success")
        .expect("row exists before success");
    assert!(before.cooldown_until.is_some());

    mark_account_sync_success(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MANUAL,
        SyncSuccessRouteState::PreserveFailureState,
    )
    .await
    .expect("mark sync success");

    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after success")
        .expect("row exists after success");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.cooldown_until.is_none());
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_402),
        "preserve-failure success should keep the last route failure marker while clearing the explicit maintenance cooldown"
    );
}

#[tokio::test]
async fn classified_sync_failure_preserves_quota_marker_from_current_syncing_row() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Quota Syncing Preserve").await;

    seed_hard_unavailable_route_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    set_account_status(&pool, account_id, UPSTREAM_ACCOUNT_STATUS_SYNCING, None)
        .await
        .expect("mark row syncing");

    let current_row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load current syncing row")
        .expect("current syncing row exists");
    assert_eq!(current_row.status, UPSTREAM_ACCOUNT_STATUS_SYNCING);

    record_classified_account_sync_failure(
        &pool,
        &current_row,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
        "usage endpoint returned 502 Bad Gateway: gateway temporarily unavailable",
    )
    .await
    .expect("record retry failure against syncing row");

    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load syncing row after retry failure")
        .expect("syncing row after retry failure exists");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some("upstream_http_5xx")
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    assert_eq!(after.last_route_failure_at, after.last_error_at);

    let summary = build_summary_from_row(
        &after,
        None,
        after.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(
        summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
}

#[tokio::test]
async fn oauth_sync_proactively_quarantines_snapshot_exhausted_account_without_prior_route_failure()
{
    let (base_url, server) = spawn_usage_snapshot_server(
        StatusCode::OK,
        json!({
            "planType": "team",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 100,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                }
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Sync Snapshot Exhausted",
        "snapshot-exhausted@example.com",
        "org_snapshot_exhausted",
        "user_snapshot_exhausted",
    )
    .await;
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");

    sync_oauth_account(&state, &row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after proactive quarantine")
        .expect("oauth row exists after proactive quarantine");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert!(after.last_successful_sync_at.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_HARD_UNAVAILABLE)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_USAGE_SNAPSHOT_EXHAUSTED)
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_USAGE_SNAPSHOT_QUOTA_EXHAUSTED)
    );
    server.abort();
}

#[tokio::test]
async fn resolver_short_circuits_when_only_persisted_snapshot_exhausted_accounts_remain() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let first = insert_api_key_account(&state.pool, "Exhausted A").await;
    let second = insert_api_key_account(&state.pool, "Exhausted B").await;
    let third = insert_api_key_account(&state.pool, "Exhausted C").await;
    let now_iso = format_utc_iso(Utc::now());
    for account_id in [first, second, third] {
        insert_limit_sample_with_usage(&state.pool, account_id, &now_iso, Some(100.0), Some(40.0))
            .await;
    }

    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve pool account");
    assert!(matches!(resolution, PoolAccountResolution::RateLimited));
}

#[tokio::test]
async fn resolver_skips_persisted_snapshot_exhausted_account_before_routing() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let exhausted = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Exhausted Candidate",
        "exhausted-candidate@example.com",
        "org_exhausted_candidate",
        "user_exhausted_candidate",
    )
    .await;
    let available = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Available Candidate",
        "available-candidate@example.com",
        "org_available_candidate",
        "user_available_candidate",
    )
    .await;
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, exhausted, &now_iso, Some(100.0), Some(20.0)).await;
    insert_limit_sample_with_usage(&state.pool, available, &now_iso, Some(42.0), Some(10.0)).await;

    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected resolver to pick an available account");
    };
    assert_eq!(account.account_id, available);
}

#[tokio::test]
async fn resolver_prefers_primary_priority_before_normal_and_fallback() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let fallback_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Fallback Priority Candidate",
        "sk-priority-fallback",
        Some("routing-priority"),
        Some("https://routing-fallback.example.com/backend-api/codex"),
    )
    .await;
    let normal_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Normal Priority Candidate",
        "sk-priority-normal",
        Some("routing-priority"),
        Some("https://routing-normal.example.com/backend-api/codex"),
    )
    .await;
    let primary_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Primary Priority Candidate",
        "sk-priority-primary",
        Some("routing-priority"),
        Some("https://routing-primary.example.com/backend-api/codex"),
    )
    .await;

    let mut fallback_rule = test_tag_routing_rule();
    fallback_rule.priority_tier = TagPriorityTier::Fallback;
    let fallback_tag = insert_tag(&state.pool, "fallback-priority", &fallback_rule)
        .await
        .expect("insert fallback tag");
    let normal_tag = insert_tag(&state.pool, "normal-priority", &test_tag_routing_rule())
        .await
        .expect("insert normal tag");
    let mut primary_rule = test_tag_routing_rule();
    primary_rule.priority_tier = TagPriorityTier::Primary;
    let primary_tag = insert_tag(&state.pool, "primary-priority", &primary_rule)
        .await
        .expect("insert primary tag");
    sync_account_tag_links(&state.pool, fallback_account_id, &[fallback_tag.summary.id])
        .await
        .expect("attach fallback tag");
    sync_account_tag_links(&state.pool, normal_account_id, &[normal_tag.summary.id])
        .await
        .expect("attach normal tag");
    sync_account_tag_links(&state.pool, primary_account_id, &[primary_tag.summary.id])
        .await
        .expect("attach primary tag");

    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(
        &state.pool,
        fallback_account_id,
        &now_iso,
        Some(1.0),
        Some(1.0),
    )
    .await;
    insert_limit_sample_with_usage(
        &state.pool,
        normal_account_id,
        &now_iso,
        Some(10.0),
        Some(1.0),
    )
    .await;
    insert_limit_sample_with_usage(
        &state.pool,
        primary_account_id,
        &now_iso,
        Some(35.0),
        Some(1.0),
    )
    .await;

    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected resolver to pick a prioritized account");
    };
    assert_eq!(account.account_id, primary_account_id);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::FreshAssignment
    );
}

#[tokio::test]
async fn resolver_keeps_higher_priority_soft_degraded_candidate_ahead_of_lower_priority_ready_account()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let slot_owner_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Node Shunt Slot Owner",
        "sk-soft-degrade-owner",
        Some("soft-degrade-priority"),
        Some("https://soft-degrade-owner.example.com/backend-api/codex"),
    )
    .await;
    let soft_degraded_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Node Shunt Soft Degraded",
        "sk-soft-degrade-target",
        Some("soft-degrade-priority"),
        Some("https://soft-degrade-target.example.com/backend-api/codex"),
    )
    .await;
    let fallback_ready_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Fallback Ready Candidate",
        "sk-soft-degrade-fallback",
        Some("soft-degrade-fallback"),
        Some("https://soft-degrade-fallback.example.com/backend-api/codex"),
    )
    .await;

    let mut primary_rule = test_tag_routing_rule();
    primary_rule.priority_tier = TagPriorityTier::Primary;
    let primary_tag = insert_tag(&state.pool, "soft-degrade-owner-primary", &primary_rule)
        .await
        .expect("insert primary owner tag");
    let normal_tag = insert_tag(
        &state.pool,
        "soft-degrade-target-normal",
        &test_tag_routing_rule(),
    )
    .await
    .expect("insert normal target tag");
    let mut fallback_rule = test_tag_routing_rule();
    fallback_rule.priority_tier = TagPriorityTier::Fallback;
    let fallback_tag = insert_tag(&state.pool, "soft-degrade-ready-fallback", &fallback_rule)
        .await
        .expect("insert fallback ready tag");
    sync_account_tag_links(&state.pool, slot_owner_id, &[primary_tag.summary.id])
        .await
        .expect("attach primary owner tag");
    sync_account_tag_links(&state.pool, soft_degraded_id, &[normal_tag.summary.id])
        .await
        .expect("attach normal target tag");
    sync_account_tag_links(&state.pool, fallback_ready_id, &[fallback_tag.summary.id])
        .await
        .expect("attach fallback ready tag");

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "soft-degrade-priority",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: vec![FORWARD_PROXY_DIRECT_KEY.to_string()],
            node_shunt_enabled: true,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save node shunt metadata");
    save_group_metadata_record_conn(
        &mut conn,
        "soft-degrade-fallback",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: vec![FORWARD_PROXY_DIRECT_KEY.to_string()],
            node_shunt_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save fallback metadata");
    drop(conn);

    let resolution =
        resolve_pool_account_for_request(&state, None, &[slot_owner_id], &HashSet::new())
            .await
            .expect("resolve soft-degraded priority candidate");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected soft-degraded candidate to remain routable");
    };
    assert_eq!(account.account_id, soft_degraded_id);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::FreshAssignment
    );
    let ForwardProxyRouteScope::BoundGroup { group_name, .. } = &account.forward_proxy_scope else {
        panic!("expected soft-degraded node shunt candidate to use bound-group live fallback");
    };
    assert_eq!(group_name, "soft-degrade-priority");
}

#[test]
fn retry_original_node_candidates_sort_after_sendable_candidates_even_when_priority_is_higher() {
    let retry_original = PoolRoutingCandidateScore {
        eligibility: PoolRoutingCandidateEligibility::SoftDegraded,
        routing_priority_rank: 0,
        capacity_lane: PoolRoutingCandidateCapacityLane::Primary,
        dispatch_state: PoolRoutingCandidateDispatchState::RetryOriginalNode,
        scarcity_score: 0.0,
        effective_load: 0,
        last_selected_at: None,
        account_id: 10,
    };
    let ready_after_migration = PoolRoutingCandidateScore {
        eligibility: PoolRoutingCandidateEligibility::SoftDegraded,
        routing_priority_rank: 2,
        capacity_lane: PoolRoutingCandidateCapacityLane::Primary,
        dispatch_state: PoolRoutingCandidateDispatchState::ReadyAfterMigration,
        scarcity_score: 0.0,
        effective_load: 0,
        last_selected_at: None,
        account_id: 11,
    };

    assert_eq!(
        compare_pool_routing_candidate_scores(&retry_original, &ready_after_migration),
        std::cmp::Ordering::Greater
    );
    assert_eq!(
        compare_pool_routing_candidate_scores(&ready_after_migration, &retry_original),
        std::cmp::Ordering::Less
    );
}

#[test]
fn overflow_candidates_sort_after_primary_candidates_even_when_priority_is_higher() {
    let overflow = PoolRoutingCandidateScore {
        eligibility: PoolRoutingCandidateEligibility::Assignable,
        routing_priority_rank: 0,
        capacity_lane: PoolRoutingCandidateCapacityLane::Overflow,
        dispatch_state: PoolRoutingCandidateDispatchState::ReadyOnOwnedNode,
        scarcity_score: 0.0,
        effective_load: 9,
        last_selected_at: None,
        account_id: 12,
    };
    let primary = PoolRoutingCandidateScore {
        eligibility: PoolRoutingCandidateEligibility::Assignable,
        routing_priority_rank: 2,
        capacity_lane: PoolRoutingCandidateCapacityLane::Primary,
        dispatch_state: PoolRoutingCandidateDispatchState::ReadyOnOwnedNode,
        scarcity_score: 0.0,
        effective_load: 1,
        last_selected_at: None,
        account_id: 13,
    };

    assert_eq!(
        compare_pool_routing_candidate_scores(&overflow, &primary),
        std::cmp::Ordering::Greater
    );
    assert_eq!(
        compare_pool_routing_candidate_scores(&primary, &overflow),
        std::cmp::Ordering::Less
    );
}

#[tokio::test]
async fn resolver_keeps_quota_exhausted_accounts_in_rate_limited_terminal_state_after_sync_block() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Quota Exhausted Resolver").await;
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    record_account_sync_recovery_blocked(
            &state.pool,
            account_id,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED,
            "manual recovery required because API key sync cannot verify whether the upstream usage limit has reset",
            Some("seed hard unavailable"),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED),
        )
        .await
        .expect("record blocked recovery");

    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve pool account");
    assert!(matches!(resolution, PoolAccountResolution::RateLimited));
}

#[tokio::test]
async fn resolver_skips_candidate_when_group_has_no_bound_proxy_keys() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let blocked = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Blocked Missing Binding",
        "blocked-missing-binding@example.com",
        "org_blocked_missing_binding",
        "user_blocked_missing_binding",
    )
    .await;
    let healthy = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Healthy Candidate",
        "healthy-candidate@example.com",
        "org_healthy_candidate",
        "user_healthy_candidate",
    )
    .await;
    set_test_account_group_name(&state.pool, blocked, Some("missing-bindings")).await;
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, blocked, &now_iso, Some(1.0), Some(1.0)).await;
    insert_limit_sample_with_usage(&state.pool, healthy, &now_iso, Some(80.0), Some(10.0)).await;

    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected resolver to skip missing-binding group and pick healthy account");
    };
    assert_eq!(account.account_id, healthy);
}

#[tokio::test]
async fn resolver_skips_candidate_when_group_has_only_unselectable_bound_proxies() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let blocked = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Blocked Unselectable Binding",
        "blocked-unselectable-binding@example.com",
        "org_blocked_unselectable_binding",
        "user_blocked_unselectable_binding",
    )
    .await;
    let healthy = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Healthy Fallback",
        "healthy-fallback@example.com",
        "org_healthy_fallback",
        "user_healthy_fallback",
    )
    .await;
    set_test_account_group_name(&state.pool, blocked, Some("staging")).await;
    upsert_test_group_binding(
        &state.pool,
        "staging",
        vec!["unselectable-bound-node".to_string()],
    )
    .await;
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, blocked, &now_iso, Some(1.0), Some(1.0)).await;
    insert_limit_sample_with_usage(&state.pool, healthy, &now_iso, Some(70.0), Some(10.0)).await;

    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected resolver to skip unselectable group and pick healthy account");
    };
    assert_eq!(account.account_id, healthy);
}

#[tokio::test]
async fn resolver_skips_ungrouped_candidate_when_healthy_grouped_account_exists() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let ungrouped = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Ungrouped Candidate",
        "ungrouped-candidate@example.com",
        "org_ungrouped_candidate",
        "user_ungrouped_candidate",
    )
    .await;
    let healthy = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Healthy Grouped Candidate",
        "healthy-grouped-candidate@example.com",
        "org_healthy_grouped_candidate",
        "user_healthy_grouped_candidate",
    )
    .await;
    set_test_account_group_name(&state.pool, ungrouped, None).await;
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, ungrouped, &now_iso, Some(1.0), Some(1.0)).await;
    insert_limit_sample_with_usage(&state.pool, healthy, &now_iso, Some(80.0), Some(10.0)).await;

    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected resolver to skip ungrouped account and pick healthy grouped account");
    };
    assert_eq!(account.account_id, healthy);
}
