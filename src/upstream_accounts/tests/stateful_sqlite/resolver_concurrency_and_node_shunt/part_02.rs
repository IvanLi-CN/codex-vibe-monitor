#[tokio::test]
pub(crate) async fn provisioning_scope_skips_proxy_keys_assigned_to_other_groups() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let secondary_proxy_key = {
        let mut manager = state.forward_proxy.lock().await;
        let settings = ForwardProxySettings {
            proxy_urls: vec!["http://127.0.0.1:18080".to_string()],
            ..Default::default()
        };
        manager.apply_settings(settings);
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
        "Provisioning Cross Group Account",
        "provision-cross-group@example.com",
        "org_provision_cross_group",
        "user_provision_cross_group",
    )
    .await;
    let occupying_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Occupying Cross Group Account",
        "occupying-cross-group@example.com",
        "org_occupying_cross_group",
        "user_occupying_cross_group",
    )
    .await;

    set_test_account_group_name(&state.pool, account_id, Some("node-shunt-provisioning-a")).await;
    set_test_account_group_name(
        &state.pool,
        occupying_account_id,
        Some("node-shunt-provisioning-b"),
    )
    .await;

    disable_provisioning_account(&state.pool, account_id).await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-provisioning-a",
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
    .expect("save provisioning group a metadata");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-provisioning-b",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: vec![FORWARD_PROXY_DIRECT_KEY.to_string()],
            node_shunt_enabled: true,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save provisioning group b metadata");
    drop(conn);

    assert_cross_group_provisioning_scope(
        &state,
        account_id,
        occupying_account_id,
        &secondary_proxy_key,
    )
    .await;
}

async fn disable_provisioning_account(pool: &SqlitePool, account_id: i64) {
    sqlx::query("UPDATE pool_upstream_accounts SET enabled = 0, updated_at = ?2 WHERE id = ?1")
        .bind(account_id)
        .bind(format_utc_iso(Utc::now()))
        .execute(pool)
        .await
        .expect("disable provisioning account");
}

async fn assert_cross_group_provisioning_scope(
    state: &AppState,
    account_id: i64,
    occupying_account_id: i64,
    secondary_proxy_key: &str,
) {
    let assignments = build_upstream_account_node_shunt_assignments(state)
        .await
        .expect("build node shunt assignments");
    assert!(!assignments.account_proxy_keys.contains_key(&account_id));
    assert_eq!(
        assignments
            .account_proxy_keys
            .get(&occupying_account_id)
            .map(String::as_str),
        Some(FORWARD_PROXY_DIRECT_KEY),
    );
    let existing_account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load provisioning account")
        .expect("provisioning account row");
    let scope = resolve_group_forward_proxy_scope_for_provisioning(
        state,
        &ResolvedRequiredGroupProxyBinding {
            group_name: "node-shunt-provisioning-a".to_string(),
            bound_proxy_keys: vec![
                FORWARD_PROXY_DIRECT_KEY.to_string(),
                secondary_proxy_key.to_string(),
            ],
            node_shunt_enabled: true,
        },
        Some(&assignments),
        Some(&existing_account),
        &HashSet::new(),
    )
    .await
    .expect("provisioning should skip proxy keys assigned to other groups");
    let ForwardProxyRouteScope::PinnedProxyKey(proxy_key) = scope else {
        panic!("expected provisioning scope to pin the remaining free node shunt slot");
    };
    assert_eq!(proxy_key, secondary_proxy_key);
}

#[tokio::test]
pub(crate) async fn provisioning_scope_skips_proxy_keys_reserved_by_other_accounts() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let secondary_proxy_key = {
        let mut manager = state.forward_proxy.lock().await;
        let settings = ForwardProxySettings {
            proxy_urls: vec!["http://127.0.0.1:18080".to_string()],
            ..Default::default()
        };
        manager.apply_settings(settings);
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
        "Provisioning Reserved Proxy Account",
        "provision-reserved@example.com",
        "org_provision_reserved",
        "user_provision_reserved",
    )
    .await;

    set_test_account_group_name(&state.pool, account_id, Some("node-shunt-provisioning")).await;

    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET enabled = 0,
                updated_at = ?2
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .expect("disable provisioning account");

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-provisioning",
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
    .expect("save node shunt provisioning metadata");
    drop(conn);

    assert_reserved_by_other_provisioning_scope(&state, account_id, &secondary_proxy_key).await;
}

async fn assert_reserved_by_other_provisioning_scope(
    state: &AppState,
    account_id: i64,
    secondary_proxy_key: &str,
) {
    let assignments = build_upstream_account_node_shunt_assignments(state)
        .await
        .expect("build node shunt assignments");
    assert!(!assignments.account_proxy_keys.contains_key(&account_id));
    state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned")
        .insert(
            "test-provisioning-live-reservation".to_string(),
            PoolRoutingReservation {
                account_id: 0,
                model: None,
                proxy_key: Some(FORWARD_PROXY_DIRECT_KEY.to_string()),
                created_at: Instant::now(),
            },
        );
    let existing_account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load existing account")
        .expect("existing account row");
    let scope = resolve_group_forward_proxy_scope_for_provisioning(
        state,
        &ResolvedRequiredGroupProxyBinding {
            group_name: "node-shunt-provisioning".to_string(),
            bound_proxy_keys: vec![
                FORWARD_PROXY_DIRECT_KEY.to_string(),
                secondary_proxy_key.to_string(),
            ],
            node_shunt_enabled: true,
        },
        Some(&assignments),
        Some(&existing_account),
        &HashSet::new(),
    )
    .await
    .expect("provisioning should skip proxy keys reserved by other accounts");
    let ForwardProxyRouteScope::PinnedProxyKey(proxy_key) = scope else {
        panic!("expected provisioning scope to pin the remaining free node shunt slot");
    };
    assert_eq!(proxy_key, secondary_proxy_key);
}

#[tokio::test]
pub(crate) async fn provisioning_scope_reuses_live_reserved_proxy_key_for_same_account() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let secondary_proxy_key = {
        let mut manager = state.forward_proxy.lock().await;
        let settings = ForwardProxySettings {
            proxy_urls: vec!["http://127.0.0.1:18080".to_string()],
            ..Default::default()
        };
        manager.apply_settings(settings);
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
        "Provisioning Reserved Self Account",
        "provision-reserved-self@example.com",
        "org_provision_reserved_self",
        "user_provision_reserved_self",
    )
    .await;

    set_test_account_group_name(&state.pool, account_id, Some("node-shunt-provisioning")).await;

    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET enabled = 0,
                updated_at = ?2
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .expect("disable provisioning account");
    let bound_proxy_keys = vec![
        FORWARD_PROXY_DIRECT_KEY.to_string(),
        secondary_proxy_key.clone(),
    ];

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-provisioning",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: bound_proxy_keys.clone(),
            node_shunt_enabled: true,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save node shunt provisioning metadata");
    drop(conn);

    assert_same_account_reuses_provisioning_scope(&state, account_id, bound_proxy_keys).await;
}

async fn assert_same_account_reuses_provisioning_scope(
    state: &AppState,
    account_id: i64,
    bound_proxy_keys: Vec<String>,
) {
    let assignments = build_upstream_account_node_shunt_assignments(state)
        .await
        .expect("build node shunt assignments");
    assert!(!assignments.account_proxy_keys.contains_key(&account_id));
    state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned")
        .insert(
            "test-provisioning-self-reservation".to_string(),
            PoolRoutingReservation {
                account_id,
                model: None,
                proxy_key: Some(FORWARD_PROXY_DIRECT_KEY.to_string()),
                created_at: Instant::now(),
            },
        );
    let existing_account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load existing account")
        .expect("existing account row");
    let scope = resolve_group_forward_proxy_scope_for_provisioning(
        state,
        &ResolvedRequiredGroupProxyBinding {
            group_name: "node-shunt-provisioning".to_string(),
            bound_proxy_keys,
            node_shunt_enabled: true,
        },
        Some(&assignments),
        Some(&existing_account),
        &HashSet::new(),
    )
    .await
    .expect("same account should reuse its live reserved proxy key");
    let ForwardProxyRouteScope::PinnedProxyKey(proxy_key) = scope else {
        panic!("expected provisioning scope to pin the same reserved node shunt slot");
    };
    assert_eq!(proxy_key, FORWARD_PROXY_DIRECT_KEY);
}

#[tokio::test]
pub(crate) async fn provisioning_scope_rejects_existing_account_without_node_shunt_slot_when_group_is_full()
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
        "Provisioned Disabled Account",
        "provision-disabled@example.com",
        "org_provision_disabled",
        "user_provision_disabled",
    )
    .await;
    let occupying_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Provisioned Occupying Account",
        "provision-occupying@example.com",
        "org_provision_occupying",
        "user_provision_occupying",
    )
    .await;

    set_test_account_group_name(&state.pool, account_id, Some("node-shunt-provisioning")).await;
    set_test_account_group_name(
        &state.pool,
        occupying_account_id,
        Some("node-shunt-provisioning"),
    )
    .await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-provisioning",
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
    .expect("save node shunt provisioning metadata");
    drop(conn);

    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET enabled = 0,
                updated_at = ?2
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .expect("disable provisioned account");

    assert_full_group_rejects_unassigned_provisioning(&state, account_id, occupying_account_id)
        .await;
}

async fn assert_full_group_rejects_unassigned_provisioning(
    state: &AppState,
    account_id: i64,
    occupying_account_id: i64,
) {
    let assignments = build_upstream_account_node_shunt_assignments(state)
        .await
        .expect("build node shunt assignments");
    assert!(!assignments.account_proxy_keys.contains_key(&account_id));
    assert_eq!(
        assignments
            .account_proxy_keys
            .get(&occupying_account_id)
            .map(String::as_str),
        Some(FORWARD_PROXY_DIRECT_KEY),
    );
    let existing_account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load existing account")
        .expect("existing account row");
    let err = resolve_group_forward_proxy_scope_for_provisioning(
        state,
        &ResolvedRequiredGroupProxyBinding {
            group_name: "node-shunt-provisioning".to_string(),
            bound_proxy_keys: test_required_group_bound_proxy_keys(),
            node_shunt_enabled: true,
        },
        Some(&assignments),
        Some(&existing_account),
        &HashSet::new(),
    )
    .await
    .expect_err("existing account without a slot should be blocked when the group is full");
    assert!(is_group_node_shunt_unassigned_message(&err.to_string()));
}

#[tokio::test]
pub(crate) async fn node_shunt_refresh_failure_reassigns_slot_within_same_request() {
    let (usage_base_url, oauth_issuer, token_requests, server) = spawn_token_failure_oauth_server(
        StatusCode::BAD_REQUEST,
        json!({
            "error": "invalid_grant",
            "error_description": "refresh token revoked"
        }),
    )
    .await;
    let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let failing_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Failing Refresh Account",
        "failing-refresh@example.com",
        "org_failing_refresh",
        "user_failing_refresh",
    )
    .await;
    let fallback_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Fallback Refresh Account",
        "fallback-refresh@example.com",
        "org_fallback_refresh",
        "user_fallback_refresh",
    )
    .await;

    set_test_account_group_name(
        &state.pool,
        failing_account_id,
        Some("node-shunt-refresh-failover"),
    )
    .await;
    set_test_account_group_name(
        &state.pool,
        fallback_account_id,
        Some("node-shunt-refresh-failover"),
    )
    .await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-refresh-failover",
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
    .expect("save node shunt refresh failover metadata");
    drop(conn);

    set_test_account_token_expires_at(
        &state.pool,
        failing_account_id,
        &format_utc_iso(Utc::now() - ChronoDuration::hours(1)),
    )
    .await;

    assert_refresh_failure_reassigns_slot(
        &state,
        failing_account_id,
        fallback_account_id,
        &token_requests,
    )
    .await;
    server.abort();
}

async fn assert_refresh_failure_reassigns_slot(
    state: &AppState,
    failing_account_id: i64,
    fallback_account_id: i64,
    token_requests: &AtomicUsize,
) {
    let resolution = resolve_pool_account_for_request(state, None, &[], &HashSet::new())
        .await
        .expect("resolve node shunt request after refresh failure");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected fallback account to be resolved after refresh failure");
    };
    assert_eq!(account.account_id, fallback_account_id);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::FreshAssignment
    );
    let ForwardProxyRouteScope::PinnedProxyKey(proxy_key) = &account.forward_proxy_scope else {
        panic!("expected fallback account to receive a pinned node shunt proxy key");
    };
    assert_eq!(proxy_key, FORWARD_PROXY_DIRECT_KEY);
    let failing_after = load_upstream_account_row(&state.pool, failing_account_id)
        .await
        .expect("load failing account after routing")
        .expect("failing account exists after routing");
    assert_eq!(failing_after.status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
    let assignments = build_upstream_account_node_shunt_assignments(state)
        .await
        .expect("build refreshed node shunt assignments");
    assert!(
        !assignments
            .account_proxy_keys
            .contains_key(&failing_account_id)
    );
    assert_eq!(
        assignments
            .account_proxy_keys
            .get(&fallback_account_id)
            .map(String::as_str),
        Some(FORWARD_PROXY_DIRECT_KEY)
    );
    assert_eq!(token_requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
pub(crate) async fn resolver_ignores_stale_sticky_routes_when_applying_concurrency_limit() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let limited_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Limited Account",
        "limited-stale@example.com",
        "org_limited_stale",
        "user_limited_stale",
    )
    .await;
    let fallback_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Fallback Account",
        "fallback-stale@example.com",
        "org_fallback_stale",
        "user_fallback_stale",
    )
    .await;

    sqlx::query("UPDATE pool_upstream_accounts SET group_name = ?2 WHERE id = ?1")
        .bind(limited_account_id)
        .bind("limited-stale")
        .execute(&state.pool)
        .await
        .expect("assign limited stale group");

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "limited-stale",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: test_required_group_bound_proxy_keys(),
            node_shunt_enabled: false,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 1,
        },
    )
    .await
    .expect("save limited stale group metadata");
    drop(conn);

    let stale_seen_at =
        format_utc_iso(Utc::now() - ChronoDuration::minutes(5) - ChronoDuration::seconds(1));
    upsert_sticky_route(
        &state.pool,
        "load-seed-stale",
        limited_account_id,
        &stale_seen_at,
    )
    .await
    .expect("seed stale sticky route");

    let resolution =
        resolve_pool_account_for_request(&state, None, &[], &std::collections::HashSet::new())
            .await
            .expect("resolve pool account");

    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected limited account to remain selectable");
    };
    assert_eq!(account.account_id, limited_account_id);
    assert_ne!(account.account_id, fallback_account_id);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::FreshAssignment
    );
}

#[tokio::test]
pub(crate) async fn resolver_allows_sticky_reuse_even_when_concurrency_limit_is_reached() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let limited_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Sticky Limited",
        "sticky-limited@example.com",
        "org_sticky_limited",
        "user_sticky_limited",
    )
    .await;

    sqlx::query("UPDATE pool_upstream_accounts SET group_name = ?2 WHERE id = ?1")
        .bind(limited_account_id)
        .bind("sticky-group")
        .execute(&state.pool)
        .await
        .expect("assign sticky group");

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "sticky-group",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: test_required_group_bound_proxy_keys(),
            node_shunt_enabled: false,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 1,
        },
    )
    .await
    .expect("save sticky group metadata");
    drop(conn);

    let now_iso = format_utc_iso(Utc::now());
    upsert_sticky_route(&state.pool, "sticky-reuse", limited_account_id, &now_iso)
        .await
        .expect("seed sticky route");

    let resolution = resolve_pool_account_for_request(
        &state,
        Some("sticky-reuse"),
        &[],
        &std::collections::HashSet::new(),
    )
    .await
    .expect("resolve sticky reuse");

    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected sticky reuse to resolve the existing account");
    };
    assert_eq!(account.account_id, limited_account_id);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::StickyReuse
    );
}

#[tokio::test]
pub(crate) async fn resolver_preserves_fallback_sticky_without_model() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let sticky_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Sticky Fallback Priority",
        "sk-sticky-priority-fallback",
        Some("sticky-priority"),
        Some("https://sticky-priority-fallback.example.com/backend-api/codex"),
    )
    .await;
    let primary_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Primary Replacement Candidate",
        "sk-sticky-priority-primary",
        Some("sticky-priority"),
        Some("https://sticky-priority-primary.example.com/backend-api/codex"),
    )
    .await;

    let mut fallback_rule = test_tag_routing_rule();
    fallback_rule.priority_tier = TagPriorityTier::Fallback;
    let fallback_tag = insert_test_tag(&state.pool, "sticky-fallback-priority", &fallback_rule)
        .await
        .expect("insert sticky fallback tag");
    let mut primary_rule = test_tag_routing_rule();
    primary_rule.priority_tier = TagPriorityTier::Primary;
    let primary_tag = insert_test_tag(&state.pool, "sticky-primary-priority", &primary_rule)
        .await
        .expect("insert sticky primary tag");
    sync_account_tag_links(&state.pool, sticky_account_id, &[fallback_tag.summary.id])
        .await
        .expect("attach sticky fallback tag");
    sync_account_tag_links(&state.pool, primary_account_id, &[primary_tag.summary.id])
        .await
        .expect("attach primary tag");

    let now_iso = format_utc_iso(Utc::now());
    upsert_sticky_route(
        &state.pool,
        "sticky-priority-reuse",
        sticky_account_id,
        &now_iso,
    )
    .await
    .expect("seed sticky route");
    insert_limit_sample_with_usage(
        &state.pool,
        sticky_account_id,
        &now_iso,
        Some(5.0),
        Some(1.0),
    )
    .await;
    insert_limit_sample_with_usage(
        &state.pool,
        primary_account_id,
        &now_iso,
        Some(1.0),
        Some(1.0),
    )
    .await;

    let resolution = resolve_pool_account_for_request(
        &state,
        Some("sticky-priority-reuse"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve sticky reuse with higher priority candidate");

    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected fallback sticky route to remain on the source without a model");
    };
    assert_eq!(account.account_id, sticky_account_id);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::StickyReuse
    );
}

#[tokio::test]
pub(crate) async fn maintenance_pass_skips_secondary_overflow_accounts_until_secondary_interval() {
    let (state, requests, server) = spawn_secondary_interval_usage_state().await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let priority_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Priority OAuth",
        "priority@example.com",
        "org_priority",
        "user_priority",
    )
    .await;
    let secondary_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Secondary OAuth",
        "secondary@example.com",
        "org_secondary",
        "user_secondary",
    )
    .await;
    save_pool_routing_maintenance_settings(
        &state.pool,
        PoolRoutingMaintenanceSettings {
            primary_sync_interval_secs: 300,
            secondary_sync_interval_secs: 1800,
            priority_available_account_cap: 1,
        },
    )
    .await
    .expect("save maintenance settings");
    insert_limit_sample_with_usage(
        &state.pool,
        priority_account_id,
        "2026-03-23T11:00:00Z",
        Some(12.0),
        Some(10.0),
    )
    .await;
    insert_limit_sample_with_usage(
        &state.pool,
        secondary_account_id,
        "2026-03-23T11:00:00Z",
        Some(14.0),
        Some(20.0),
    )
    .await;
    let last_synced_at = format_utc_iso(Utc::now() - ChronoDuration::minutes(10));
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET last_synced_at = ?3,
                last_successful_sync_at = ?3
            WHERE id IN (?1, ?2)
            "#,
    )
    .bind(priority_account_id)
    .bind(secondary_account_id)
    .bind(&last_synced_at)
    .execute(&state.pool)
    .await
    .expect("seed sync times");

    run_upstream_account_maintenance_once(state.clone())
        .await
        .expect("run maintenance pass");
    timeout(Duration::from_secs(8), async {
        while requests.load(Ordering::SeqCst) < 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("priority maintenance request should finish");
    tokio::time::sleep(Duration::from_millis(150)).await;

    assert_eq!(
        requests.load(Ordering::SeqCst),
        1,
        "overflow secondary account should not sync on the primary interval"
    );
    server.abort();
}

async fn spawn_secondary_interval_usage_state()
-> (Arc<AppState>, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
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
    let state = test_app_state_with_usage_base(&format!("http://{addr}/backend-api")).await;
    (state, requests, server)
}

use super::*;
