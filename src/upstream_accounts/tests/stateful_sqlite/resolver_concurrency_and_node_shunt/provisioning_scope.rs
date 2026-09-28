use super::*;

#[tokio::test]
async fn provisioning_scope_reuses_existing_account_node_shunt_slot_when_group_is_full() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Provisioned Existing Account",
        "provision-existing@example.com",
        "org_provision_existing",
        "user_provision_existing",
    )
    .await;

    set_test_account_group_name(&state.pool, account_id, Some("node-shunt-provisioning")).await;

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

    let assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
        .await
        .expect("build node shunt assignments");
    let existing_account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load existing account")
        .expect("existing account row");

    let scope = resolve_group_forward_proxy_scope_for_provisioning(
        state.as_ref(),
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
    .expect("reuse existing node shunt slot");

    let ForwardProxyRouteScope::PinnedProxyKey(proxy_key) = scope else {
        panic!("expected provisioning scope to pin the existing node shunt slot");
    };
    assert_eq!(proxy_key, FORWARD_PROXY_DIRECT_KEY);
}

#[tokio::test]
async fn provisioning_scope_claims_free_node_for_existing_account_without_assigned_slot() {
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

    set_test_account_group_name(&state.pool, account_id, Some("node-shunt-provisioning")).await;

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

    let assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
        .await
        .expect("build node shunt assignments");
    assert!(
        !assignments.account_proxy_keys.contains_key(&account_id),
        "disabled account should not occupy a node shunt slot",
    );
    let existing_account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load existing account")
        .expect("existing account row");

    let scope = resolve_group_forward_proxy_scope_for_provisioning(
        state.as_ref(),
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
    .expect("existing account should be able to claim a free node shunt slot");

    let ForwardProxyRouteScope::PinnedProxyKey(proxy_key) = scope else {
        panic!("expected provisioning scope to pin a free node shunt slot");
    };
    assert_eq!(proxy_key, FORWARD_PROXY_DIRECT_KEY);
}

#[tokio::test]
async fn provisioning_scope_skips_proxy_keys_assigned_to_other_groups() {
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

    let assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
        .await
        .expect("build node shunt assignments");
    assert!(
        !assignments.account_proxy_keys.contains_key(&account_id),
        "disabled provisioning account should not occupy a node shunt slot",
    );
    assert_eq!(
        assignments
            .account_proxy_keys
            .get(&occupying_account_id)
            .map(String::as_str),
        Some(FORWARD_PROXY_DIRECT_KEY),
        "other group should already occupy the shared direct node",
    );
    let existing_account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load provisioning account")
        .expect("provisioning account row");

    let scope = resolve_group_forward_proxy_scope_for_provisioning(
        state.as_ref(),
        &ResolvedRequiredGroupProxyBinding {
            group_name: "node-shunt-provisioning-a".to_string(),
            bound_proxy_keys: vec![
                FORWARD_PROXY_DIRECT_KEY.to_string(),
                secondary_proxy_key.clone(),
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
async fn provisioning_scope_skips_proxy_keys_reserved_by_other_accounts() {
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

    let assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
        .await
        .expect("build node shunt assignments");
    assert!(
        !assignments.account_proxy_keys.contains_key(&account_id),
        "disabled account should not occupy a node shunt slot",
    );
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
        state.as_ref(),
        &ResolvedRequiredGroupProxyBinding {
            group_name: "node-shunt-provisioning".to_string(),
            bound_proxy_keys: vec![
                FORWARD_PROXY_DIRECT_KEY.to_string(),
                secondary_proxy_key.clone(),
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
async fn provisioning_scope_reuses_live_reserved_proxy_key_for_same_account() {
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

    let assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
        .await
        .expect("build node shunt assignments");
    assert!(
        !assignments.account_proxy_keys.contains_key(&account_id),
        "disabled account should not occupy a node shunt slot",
    );
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
        state.as_ref(),
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
async fn provisioning_scope_rejects_existing_account_without_node_shunt_slot_when_group_is_full() {
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

    let assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
        .await
        .expect("build node shunt assignments");
    assert!(
        !assignments.account_proxy_keys.contains_key(&account_id),
        "disabled account should not occupy a node shunt slot",
    );
    assert_eq!(
        assignments
            .account_proxy_keys
            .get(&occupying_account_id)
            .map(String::as_str),
        Some(FORWARD_PROXY_DIRECT_KEY),
        "eligible peer should occupy the only available node shunt slot",
    );
    let existing_account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load existing account")
        .expect("existing account row");

    let err = resolve_group_forward_proxy_scope_for_provisioning(
        state.as_ref(),
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
