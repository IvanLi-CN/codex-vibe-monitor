#[tokio::test]
pub(crate) async fn standalone_search_override_is_rejected_for_oauth_accounts() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_oauth_account(&state.pool, "OAuth Search Override Rejected").await;

    let err = state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            account_id,
            UpdateUpstreamAccountRequest {
                standalone_search_capability_override: OptionalField::Value(
                    "supported".to_string(),
                ),
                ..UpdateUpstreamAccountRequest::default()
            },
        )
        .await
        .expect_err("OAuth accounts must reject standalone search overrides");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(err.1.contains("only supported for API key accounts"));

    let err = state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            account_id,
            UpdateUpstreamAccountRequest {
                standalone_search_capability_override: OptionalField::Null,
                ..UpdateUpstreamAccountRequest::default()
            },
        )
        .await
        .expect_err("OAuth accounts must reject clearing standalone search overrides");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);

    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load OAuth account after rejected override")
        .expect("OAuth account exists after rejected override");
    assert_eq!(row.policy_standalone_search_capability_override, None);
}

#[tokio::test]
pub(crate) async fn image_intent_explicit_unsupported_failure_learns_unsupported_capability() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Image Failure Learns Unsupported").await;

    record_pool_route_http_failure_for_endpoint_with_image_intent(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        false,
        Some("sticky-image-unsupported"),
        StatusCode::BAD_REQUEST,
        "pool upstream responded with 400: unsupported tool: image_generation is not supported by this account",
        Some("invk_image_unsupported"),
        "/v1/responses",
        ImageIntent::Yes,
    )
    .await
    .expect("record explicit unsupported image failure");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after unsupported image failure")
        .expect("row exists after unsupported image failure");
    assert_eq!(row.response_endpoint_capability.as_deref(), Some("unknown"));
    assert_eq!(
        row.response_image_tool_capability.as_deref(),
        Some("unsupported")
    );

    let direct_account_id =
        insert_oauth_account(&pool, "Direct Image Failure Learns Unsupported").await;
    record_pool_route_http_failure_for_endpoint_with_image_intent(
        &pool,
        direct_account_id,
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        false,
        Some("sticky-direct-image-unsupported"),
        StatusCode::BAD_REQUEST,
        "pool upstream responded with 400: No available channel for model gpt-image-1 under group default",
        Some("invk_direct_image_unsupported"),
        "/v1/images/generations",
        ImageIntent::DirectImage,
    )
    .await
    .expect("record explicit unsupported direct image failure");

    let direct_row = load_upstream_account_row(&pool, direct_account_id)
        .await
        .expect("load row after unsupported direct image failure")
        .expect("row exists after unsupported direct image failure");
    assert_eq!(
        direct_row.image_endpoint_capability.as_deref(),
        Some("unsupported")
    );
}

#[tokio::test]
pub(crate) async fn stale_capability_observations_cannot_overwrite_newer_results() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Fenced Capability Observations").await;
    let newer = "2026-09-13T10:00:01.000Z";
    let older = "2026-09-13T10:00:00.000Z";

    record_capability_observation_with_observed_at(
        &pool,
        account_id,
        UpstreamCapabilityAxis::ResponseEndpoint,
        CapabilitySupport::Supported,
        Some("newer success"),
        Some(newer),
    )
    .await
    .expect("record newer capability observation");
    record_capability_observation_with_observed_at(
        &pool,
        account_id,
        UpstreamCapabilityAxis::ResponseEndpoint,
        CapabilitySupport::Unsupported,
        Some("older failure"),
        Some(older),
    )
    .await
    .expect("record stale capability observation");

    record_compact_support_observation_with_observed_at(
        &pool,
        account_id,
        COMPACT_SUPPORT_STATUS_SUPPORTED,
        Some("newer success"),
        Some(newer),
    )
    .await
    .expect("record newer compact observation");
    record_compact_support_observation_with_observed_at(
        &pool,
        account_id,
        COMPACT_SUPPORT_STATUS_UNSUPPORTED,
        Some("older failure"),
        Some(older),
    )
    .await
    .expect("record stale compact observation");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load fenced observation row")
        .expect("fenced observation row exists");
    assert_eq!(
        row.response_endpoint_capability.as_deref(),
        Some(CapabilitySupport::Supported.as_str())
    );
    assert_eq!(
        row.response_endpoint_capability_observed_at.as_deref(),
        Some(newer)
    );
    assert_eq!(
        row.compact_support_status.as_deref(),
        Some(COMPACT_SUPPORT_STATUS_SUPPORTED)
    );
    assert_eq!(row.compact_support_observed_at.as_deref(), Some(newer));
}

#[tokio::test]
pub(crate) async fn image_intent_validation_failure_does_not_learn_unsupported_capability() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Image Validation Failure Keeps Unknown").await;

    record_pool_route_http_failure_for_endpoint_with_image_intent(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        false,
        Some("sticky-image-invalid-payload"),
        StatusCode::BAD_REQUEST,
        "pool upstream responded with 400: invalid image size: width must be divisible by 64",
        Some("invk_image_invalid_payload"),
        "/v1/responses",
        ImageIntent::Yes,
    )
    .await
    .expect("record image validation failure");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after image validation failure")
        .expect("row exists after image validation failure");
    assert_eq!(row.response_endpoint_capability.as_deref(), Some("unknown"));
    assert_eq!(
        row.response_image_tool_capability.as_deref(),
        Some("unknown")
    );
}

#[tokio::test]
pub(crate) async fn mark_account_sync_success_preserves_route_cooldown_state() {
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
pub(crate) async fn mark_account_sync_success_clears_hard_unavailable_state_when_requested() {
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
pub(crate) async fn sync_api_key_account_preserves_route_cooldown_state() {
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
pub(crate) async fn sync_api_key_account_keeps_hard_unavailable_accounts_blocked() {
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
pub(crate) async fn sync_scope_reuses_live_reserved_node_for_same_account_before_shared_group_probe()
 {
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
pub(crate) async fn oauth_sync_refresh_due_reuses_sync_only_scope_for_token_refresh() {
    let (proxy_url, usage_requests, token_requests, server) =
        spawn_proxy_only_oauth_sync_server().await;
    let state = test_app_state_with_usage_and_oauth_base(
        "http://unreachable.invalid/backend-api",
        "http://unreachable.invalid",
    )
    .await;
    let account_id = seed_refresh_due_sync_scope(&state, proxy_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");

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

async fn seed_refresh_due_sync_scope(state: &AppState, proxy_url: String) -> i64 {
    let secondary_proxy_key = {
        let mut manager = state.forward_proxy.lock().await;
        manager.apply_settings(ForwardProxySettings {
            proxy_urls: vec![proxy_url],
            ..Default::default()
        });
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
    set_test_account_token_expires_at(
        &state.pool,
        account_id,
        &format_utc_iso(Utc::now() - ChronoDuration::minutes(5)),
    )
    .await;
    account_id
}

#[tokio::test]
pub(crate) async fn sync_scope_falls_back_to_shared_bound_group_when_exclusive_slot_is_full() {
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

async fn seed_group_node_shunt_unassigned_accounts(
    state: &AppState,
    group_name: &str,
    occupying_display_name: &str,
    queued_display_name: &str,
) -> (i64, i64) {
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let occupying_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        occupying_display_name,
        "occupying@example.com",
        "org_occupying",
        "user_occupying",
    )
    .await;
    let queued_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        queued_display_name,
        "queued@example.com",
        "org_queued",
        "user_queued",
    )
    .await;
    for account_id in [occupying_account_id, queued_account_id] {
        set_test_account_group_name(&state.pool, account_id, Some(group_name)).await;
    }
    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        group_name,
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
    let assignments = build_upstream_account_node_shunt_assignments(state)
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
            .contains_key(&queued_account_id)
    );
    (occupying_account_id, queued_account_id)
}

#[tokio::test]
pub(crate) async fn manual_sync_allows_group_node_shunt_unassigned_account_to_probe_bound_node() {
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
    let (_occupying_account_id, queued_account_id) = seed_group_node_shunt_unassigned_accounts(
        &state,
        "node-shunt-sync",
        "Occupying OAuth",
        "Queued OAuth",
    )
    .await;

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
pub(crate) async fn maintenance_sync_allows_group_node_shunt_unassigned_account_to_probe_bound_node()
 {
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
    let (_occupying_account_id, queued_account_id) = seed_group_node_shunt_unassigned_accounts(
        &state,
        "node-shunt-maint",
        "Occupying Maintenance OAuth",
        "Queued Maintenance OAuth",
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
pub(crate) async fn bulk_sync_allows_group_node_shunt_unassigned_account_to_probe_bound_node() {
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
    let (_occupying_account_id, queued_account_id) = seed_group_node_shunt_unassigned_accounts(
        &state,
        "node-shunt-bulk",
        "Occupying Bulk OAuth",
        "Queued Bulk OAuth",
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

use super::*;
