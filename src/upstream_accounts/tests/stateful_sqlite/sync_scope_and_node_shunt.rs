use super::*;
use serde_json::json;
use std::time::{Duration, Instant};
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
            single_account_rotation_enabled: false,
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
                model: None,
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
        let settings = ForwardProxySettings {
            proxy_urls: vec![proxy_url],
            ..Default::default()
        };
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
            single_account_rotation_enabled: false,
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
                model: None,
                proxy_key: Some(secondary_proxy_key),
                created_at: Instant::now(),
            },
        );
    set_test_account_access_token(
        &state.pool,
        crypto_key,
        account_id,
        test_access_token_expiring_at(Utc::now() - ChronoDuration::minutes(5)),
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
    assert_eq!(
        credentials.refresh_token.as_deref(),
        Some("proxy-refreshed-refresh-token")
    );

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
            single_account_rotation_enabled: false,
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
                    "resetsAt": 1771322400}, "secondaryWindow": {"usedPercent": 42, "windowDurationMins": 10080, "resetsAt": 1771927200}
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
            single_account_rotation_enabled: false,
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
                    "resetsAt": 1771322400}, "secondaryWindow": {"usedPercent": 42, "windowDurationMins": 10080, "resetsAt": 1771927200}
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
            single_account_rotation_enabled: false,
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
                    "resetsAt": 1771322400}, "secondaryWindow": {"usedPercent": 42, "windowDurationMins": 10080, "resetsAt": 1771927200}
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
            single_account_rotation_enabled: false,
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
    let terminal = timeout(Duration::from_secs(15), async {
        loop {
            if let Some(terminal) = job.terminal_event.lock().await.clone() {
                return terminal;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("bulk sync job should finish within timeout");
    let BulkUpstreamAccountSyncTerminalEvent::Completed(payload) = terminal else {
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
            single_account_rotation_enabled: false,
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
            single_account_rotation_enabled: false,
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
        &state.config,
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
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Recoverable API Key",
        "recoverable-api-key",
        None,
        Some("https://recoverable-api-key.example.com/backend-api/codex"),
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
    let mut availability = state.pool_routing_availability.subscribe();

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
                note: None,
                group_note: None,
                concurrency_limit: None,
                upstream_base_url: OptionalField::Missing,
                bound_proxy_keys: OptionalField::Missing,
                enabled: None,
                is_mother: None,
                api_key: Some("sk-live-new".to_string()),
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: None,
                routing_rule: None,
                ..UpdateUpstreamAccountRequest::default()
            },
        )
        .await
        .expect("update api key account");
    tokio::time::timeout(Duration::from_secs(1), availability.changed())
        .await
        .expect("manual recovery should publish routing availability")
        .expect("routing availability signal should stay open");

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
async fn successful_account_sync_publishes_routing_availability_on_recovery() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Sync Recovery Signal",
        "sync-recovery-signal-key",
        None,
        Some("https://sync-recovery-signal.example.com/backend-api/codex"),
    )
    .await;
    set_account_status(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        Some("temporary sync failure"),
    )
    .await
    .expect("make account unavailable before sync");
    let mut availability = state.pool_routing_availability.subscribe();

    sync_upstream_account_by_id(state.as_ref(), account_id, SyncCause::Manual)
        .await
        .expect("sync should recover the account");
    tokio::time::timeout(Duration::from_secs(1), availability.changed())
        .await
        .expect("sync recovery should publish routing availability")
        .expect("routing availability signal should stay open");

    let recovered = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load recovered account")
        .expect("recovered account exists");
    assert_eq!(recovered.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(is_account_selectable_for_fresh_assignment(
        &recovered,
        false,
        Utc::now()
    ));
}

#[tokio::test]
async fn imported_oauth_probe_publishes_routing_availability_on_recovery() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Imported OAuth Recovery Signal",
        "import-recovery@example.com",
        "org_import_recovery",
        "user_import_recovery",
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
    let mut availability = state.pool_routing_availability.subscribe();
    let probe = ImportedOauthProbeOutcome {
        token_expires_at: format_utc_iso(Utc::now() + ChronoDuration::days(30)),
        refresh_token_expires_at: None,
        refresh_token_expiry_source: "not_provided".to_string(),
        refresh_token_metadata_update: true,
        credentials: StoredOauthCredentials {
            access_token: "imported-access-token".to_string(),
            refresh_token: Some("imported-refresh-token".to_string()),
            id_token: test_id_token(
                "import-recovery@example.com",
                Some("org_import_recovery"),
                Some("user_import_recovery"),
                Some("team"),
            ),
            token_type: Some("Bearer".to_string()),
        },
        claims: ChatgptJwtClaims::default(),
        usage_snapshot: Some(NormalizedUsageSnapshot {
            plan_type: Some("team".to_string()),
            limit_id: "codex".to_string(),
            limit_name: Some("Codex".to_string()),
            primary: None,
            secondary: None,
            credits: None,
            ..Default::default()
        }),
        maintenance_proxy_snapshot: None,
        exhausted: false,
        usage_snapshot_warning: None,
    };

    apply_imported_oauth_probe_result(state.as_ref(), account_id, &probe)
        .await
        .expect("apply imported OAuth recovery probe");
    tokio::time::timeout(Duration::from_secs(1), availability.changed())
        .await
        .expect("OAuth import recovery should publish routing availability")
        .expect("routing availability signal should stay open");

    let recovered = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load imported OAuth account")
        .expect("imported OAuth account exists");
    assert!(is_account_selectable_for_fresh_assignment(
        &recovered,
        false,
        Utc::now()
    ));

    let unchanged_availability = state.pool_routing_availability.subscribe();
    apply_imported_oauth_probe_result(state.as_ref(), account_id, &probe)
        .await
        .expect("reapply imported OAuth probe to an already routable account");
    assert!(
        !unchanged_availability
            .has_changed()
            .expect("read availability"),
        "an already routable account must not publish a redundant availability event"
    );
}
