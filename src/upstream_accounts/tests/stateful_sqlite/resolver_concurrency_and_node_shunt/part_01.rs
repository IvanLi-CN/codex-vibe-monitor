#[tokio::test]
pub(crate) async fn resolver_skips_account_when_effective_concurrency_limit_is_reached() {
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
        "limited@example.com",
        "org_limited",
        "user_limited",
    )
    .await;
    let fallback_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Fallback Account",
        "fallback@example.com",
        "org_fallback",
        "user_fallback",
    )
    .await;

    sqlx::query("UPDATE pool_upstream_accounts SET group_name = ?2 WHERE id = ?1")
        .bind(limited_account_id)
        .bind("limited")
        .execute(&state.pool)
        .await
        .expect("assign limited group");

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "limited",
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
    .expect("save limited group metadata");
    drop(conn);

    let now_iso = format_utc_iso(Utc::now());
    upsert_sticky_route(&state.pool, "load-seed", limited_account_id, &now_iso)
        .await
        .expect("seed active sticky route");

    let resolution =
        resolve_pool_account_for_request(&state, None, &[], &std::collections::HashSet::new())
            .await
            .expect("resolve pool account");

    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected fallback account to be selected");
    };
    assert_eq!(account.account_id, fallback_account_id);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::FreshAssignment
    );
}

#[tokio::test]
pub(crate) async fn latest_usage_sample_map_keeps_latest_non_empty_sample_plan_type() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Plan Type Sample Account",
        "plan-sample@example.com",
        "org_plan_sample",
        "user_plan_sample",
    )
    .await;

    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET plan_type = 'pro',
                plan_type_observed_at = '2026-03-14T00:00:00Z'
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("seed account plan type");

    sqlx::query(
        r#"
            INSERT INTO pool_upstream_account_limit_samples (
                account_id, captured_at, limit_id, limit_name, plan_type,
                primary_used_percent, primary_window_minutes, primary_resets_at,
                secondary_used_percent, secondary_window_minutes, secondary_resets_at,
                credits_has_credits, credits_unlimited, credits_balance
            ) VALUES (
                ?1, '2026-03-15T00:00:00Z', NULL, NULL, 'team',
                24.0, 300, '2026-03-15T05:00:00Z',
                18.0, 10080, '2026-03-22T00:00:00Z',
                1, 0, '4.20'
            ), (
                ?1, '2026-03-16T00:00:00Z', NULL, NULL, NULL,
                32.0, 300, '2026-03-16T05:00:00Z',
                27.0, 10080, '2026-03-23T00:00:00Z',
                1, 0, '3.80'
            )
            "#,
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("insert usage samples");

    let sample = load_latest_usage_sample(&state.pool, account_id)
        .await
        .expect("load latest sample")
        .expect("sample exists");

    assert_eq!(sample.captured_at, "2026-03-16T00:00:00Z");
    assert_eq!(sample.plan_type.as_deref(), Some("team"));
    assert_eq!(sample.primary_used_percent, Some(32.0));
    assert_eq!(sample.secondary_used_percent, Some(27.0));
    assert_eq!(sample.credits_balance.as_deref(), Some("3.80"));
}

#[tokio::test]
pub(crate) async fn node_shunt_assignments_preserve_slots_for_accounts_with_in_flight_reservations()
{
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
    let available_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Available Slot Account",
        "available-slot@example.com",
        "org_available_slot",
        "user_available_slot",
    )
    .await;
    let reserved_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Reserved Slot Account",
        "reserved-slot@example.com",
        "org_reserved_slot",
        "user_reserved_slot",
    )
    .await;
    let overflow_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Overflow Slot Account",
        "overflow-slot@example.com",
        "org_overflow_slot",
        "user_overflow_slot",
    )
    .await;

    set_test_account_group_name(
        &state.pool,
        available_account_id,
        Some("node-shunt-priority"),
    )
    .await;
    set_test_account_group_name(
        &state.pool,
        reserved_account_id,
        Some("node-shunt-priority"),
    )
    .await;
    set_test_account_group_name(
        &state.pool,
        overflow_account_id,
        Some("node-shunt-priority"),
    )
    .await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-priority",
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
    .expect("save node shunt metadata");
    drop(conn);

    assert_in_flight_node_shunt_assignments(
        &state,
        available_account_id,
        reserved_account_id,
        overflow_account_id,
        &secondary_proxy_key,
    )
    .await;
}

async fn assert_in_flight_node_shunt_assignments(
    state: &AppState,
    available_account_id: i64,
    reserved_account_id: i64,
    overflow_account_id: i64,
    secondary_proxy_key: &str,
) {
    let initial_assignments = build_upstream_account_node_shunt_assignments(state)
        .await
        .expect("build initial node shunt assignments");
    assert_eq!(
        initial_assignments
            .account_proxy_keys
            .get(&available_account_id)
            .map(String::as_str),
        Some(FORWARD_PROXY_DIRECT_KEY)
    );
    assert_eq!(
        initial_assignments
            .account_proxy_keys
            .get(&reserved_account_id)
            .map(String::as_str),
        Some(secondary_proxy_key)
    );
    state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned")
        .insert(
            "test-node-shunt-reservation".to_string(),
            PoolRoutingReservation {
                account_id: reserved_account_id,
                model: None,
                proxy_key: Some(secondary_proxy_key.to_string()),
                created_at: Instant::now(),
            },
        );
    set_test_account_group_name(&state.pool, available_account_id, None).await;
    let assignments = build_upstream_account_node_shunt_assignments(state)
        .await
        .expect("build node shunt assignments");
    assert_eq!(
        assignments
            .account_proxy_keys
            .get(&reserved_account_id)
            .map(String::as_str),
        Some(secondary_proxy_key)
    );
    assert!(
        assignments
            .account_proxy_keys
            .get(&overflow_account_id)
            .is_some_and(|proxy_key| proxy_key == FORWARD_PROXY_DIRECT_KEY)
    );
    assert!(
        assignments
            .eligible_account_ids
            .contains(&reserved_account_id)
    );
    assert!(
        assignments
            .eligible_account_ids
            .contains(&overflow_account_id)
    );
}

#[tokio::test]
pub(crate) async fn node_shunt_assignments_keep_all_reserved_proxy_keys_occupied_for_one_account() {
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
    let reserved_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Reserved Multi Slot Account",
        "reserved-multi-slot@example.com",
        "org_reserved_multi_slot",
        "user_reserved_multi_slot",
    )
    .await;
    let overflow_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Overflow Multi Slot Account",
        "overflow-multi-slot@example.com",
        "org_overflow_multi_slot",
        "user_overflow_multi_slot",
    )
    .await;

    set_test_account_group_name(
        &state.pool,
        reserved_account_id,
        Some("node-shunt-multi-reserved"),
    )
    .await;
    set_test_account_group_name(
        &state.pool,
        overflow_account_id,
        Some("node-shunt-multi-reserved"),
    )
    .await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-multi-reserved",
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
    .expect("save node shunt metadata");
    drop(conn);

    seed_multi_reserved_node_shunt_reservations(&state, reserved_account_id, &secondary_proxy_key);

    assert_multi_reserved_node_shunt_assignments(
        &state,
        reserved_account_id,
        overflow_account_id,
        &secondary_proxy_key,
    )
    .await;
}

fn seed_multi_reserved_node_shunt_reservations(
    state: &AppState,
    account_id: i64,
    secondary_proxy_key: &str,
) {
    let mut reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    for (key, proxy_key) in [
        (
            "test-node-shunt-reservation-direct",
            FORWARD_PROXY_DIRECT_KEY,
        ),
        ("test-node-shunt-reservation-secondary", secondary_proxy_key),
    ] {
        reservations.insert(
            key.to_string(),
            PoolRoutingReservation {
                account_id,
                model: None,
                proxy_key: Some(proxy_key.to_string()),
                created_at: Instant::now(),
            },
        );
    }
}

async fn assert_multi_reserved_node_shunt_assignments(
    state: &AppState,
    reserved_account_id: i64,
    overflow_account_id: i64,
    secondary_proxy_key: &str,
) {
    let assignments = build_upstream_account_node_shunt_assignments(state)
        .await
        .expect("build node shunt assignments");
    assert!(
        assignments
            .account_proxy_keys
            .get(&reserved_account_id)
            .is_some_and(|proxy_key| proxy_key == FORWARD_PROXY_DIRECT_KEY
                || proxy_key == secondary_proxy_key)
    );
    assert!(
        !assignments
            .account_proxy_keys
            .contains_key(&overflow_account_id)
    );
    assert_eq!(
        assignments
            .group_assigned_proxy_keys
            .get("node-shunt-multi-reserved")
            .map(|keys| keys.len()),
        Some(2)
    );
}

#[tokio::test]
pub(crate) async fn node_shunt_assignments_prefer_primary_priority_before_fallback() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let fallback_account_id =
        insert_test_pool_oauth_account(&state, "Node Shunt Fallback", "oauth-node-shunt-fallback")
            .await;
    let primary_account_id =
        insert_test_pool_oauth_account(&state, "Node Shunt Primary", "oauth-node-shunt-primary")
            .await;
    set_test_account_group_name(
        &state.pool,
        fallback_account_id,
        Some("node-shunt-priority"),
    )
    .await;
    set_test_account_group_name(&state.pool, primary_account_id, Some("node-shunt-priority")).await;

    let mut fallback_rule = test_tag_routing_rule();
    fallback_rule.priority_tier = TagPriorityTier::Fallback;
    let fallback_tag = insert_test_tag(&state.pool, "node-shunt-fallback", &fallback_rule)
        .await
        .expect("insert fallback tag");
    let mut primary_rule = test_tag_routing_rule();
    primary_rule.priority_tier = TagPriorityTier::Primary;
    let primary_tag = insert_test_tag(&state.pool, "node-shunt-primary", &primary_rule)
        .await
        .expect("insert primary tag");
    sync_account_tag_links(&state.pool, fallback_account_id, &[fallback_tag.summary.id])
        .await
        .expect("attach fallback tag");
    sync_account_tag_links(&state.pool, primary_account_id, &[primary_tag.summary.id])
        .await
        .expect("attach primary tag");

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-priority",
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
    .expect("save node shunt metadata");
    drop(conn);

    let assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
        .await
        .expect("build node shunt assignments");

    assert_eq!(
        assignments
            .account_proxy_keys
            .get(&primary_account_id)
            .map(String::as_str),
        Some(FORWARD_PROXY_DIRECT_KEY)
    );
    assert!(
        !assignments
            .account_proxy_keys
            .contains_key(&fallback_account_id)
    );
}

#[tokio::test]
pub(crate) async fn node_shunt_assignments_keep_globally_reserved_proxy_keys_occupied() {
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
        "Globally Reserved Proxy Account",
        "globally-reserved@example.com",
        "org_globally_reserved",
        "user_globally_reserved",
    )
    .await;
    let overflow_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Overflow Globally Reserved Proxy Account",
        "overflow-globally-reserved@example.com",
        "org_overflow_globally_reserved",
        "user_overflow_globally_reserved",
    )
    .await;

    set_test_account_group_name(
        &state.pool,
        account_id,
        Some("node-shunt-global-reservation"),
    )
    .await;
    set_test_account_group_name(
        &state.pool,
        overflow_account_id,
        Some("node-shunt-global-reservation"),
    )
    .await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-global-reservation",
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
    .expect("save node shunt metadata");
    drop(conn);

    state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned")
        .insert(
            "test-node-shunt-global-reservation".to_string(),
            PoolRoutingReservation {
                account_id: 0,
                model: None,
                proxy_key: Some(FORWARD_PROXY_DIRECT_KEY.to_string()),
                created_at: Instant::now(),
            },
        );

    assert_global_reserved_node_shunt_assignments(
        &state,
        account_id,
        overflow_account_id,
        &secondary_proxy_key,
    )
    .await;
}

async fn assert_global_reserved_node_shunt_assignments(
    state: &AppState,
    account_id: i64,
    overflow_account_id: i64,
    secondary_proxy_key: &str,
) {
    let assignments = build_upstream_account_node_shunt_assignments(state)
        .await
        .expect("build node shunt assignments");
    assert_eq!(
        assignments
            .account_proxy_keys
            .get(&account_id)
            .map(String::as_str),
        Some(secondary_proxy_key)
    );
    assert!(
        !assignments
            .account_proxy_keys
            .contains_key(&overflow_account_id)
    );
    assert!(
        assignments
            .group_assigned_proxy_keys
            .get("node-shunt-global-reservation")
            .is_some_and(|proxy_keys| {
                proxy_keys.contains(FORWARD_PROXY_DIRECT_KEY)
                    && proxy_keys.contains(secondary_proxy_key)
            })
    );
}

#[tokio::test]
pub(crate) async fn node_shunt_assignments_keep_shared_proxy_keys_exclusive_across_groups() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let group_a_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Group A Shared Node Account",
        "group-a-shared@example.com",
        "org_group_a_shared",
        "user_group_a_shared",
    )
    .await;
    let group_b_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Group B Shared Node Account",
        "group-b-shared@example.com",
        "org_group_b_shared",
        "user_group_b_shared",
    )
    .await;

    set_test_account_group_name(&state.pool, group_a_account_id, Some("shared-node-group-a")).await;
    set_test_account_group_name(&state.pool, group_b_account_id, Some("shared-node-group-b")).await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    for group_name in ["shared-node-group-a", "shared-node-group-b"] {
        save_group_metadata_record_conn(
            &mut conn,
            group_name,
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
        .expect("save shared node shunt metadata");
    }
    drop(conn);

    let assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
        .await
        .expect("build node shunt assignments");

    let assigned_account_ids = assignments
        .account_proxy_keys
        .iter()
        .filter_map(|(account_id, proxy_key)| {
            (proxy_key == FORWARD_PROXY_DIRECT_KEY).then_some(*account_id)
        })
        .collect::<HashSet<_>>();

    assert_eq!(assigned_account_ids.len(), 1);
    assert!(
        assigned_account_ids.contains(&group_a_account_id)
            || assigned_account_ids.contains(&group_b_account_id)
    );
}

#[tokio::test]
pub(crate) async fn node_shunt_sticky_reuse_preserves_slot_for_in_flight_account() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let reserved_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Reserved Sticky Account",
        "reserved-sticky@example.com",
        "org_reserved_sticky",
        "user_reserved_sticky",
    )
    .await;
    let available_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Available Sticky Account",
        "available-sticky@example.com",
        "org_available_sticky",
        "user_available_sticky",
    )
    .await;

    set_test_account_group_name(&state.pool, reserved_account_id, Some("node-shunt-sticky")).await;
    set_test_account_group_name(&state.pool, available_account_id, Some("node-shunt-sticky")).await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-sticky",
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
    .expect("save node shunt sticky metadata");
    drop(conn);

    let now_iso = format_utc_iso(Utc::now());
    upsert_sticky_route(
        &state.pool,
        "node-shunt-sticky-reuse",
        reserved_account_id,
        &now_iso,
    )
    .await
    .expect("seed sticky route");

    state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned")
        .insert(
            "test-node-shunt-sticky-reservation".to_string(),
            PoolRoutingReservation {
                account_id: reserved_account_id,
                model: None,
                proxy_key: Some(FORWARD_PROXY_DIRECT_KEY.to_string()),
                created_at: Instant::now(),
            },
        );

    let resolution = resolve_pool_account_for_request(
        &state,
        Some("node-shunt-sticky-reuse"),
        &[],
        &std::collections::HashSet::new(),
    )
    .await
    .expect("resolve node shunt sticky reuse");

    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected node shunt sticky reuse to resolve the reserved account");
    };
    assert_eq!(account.account_id, reserved_account_id);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::StickyReuse
    );
}

#[tokio::test]
pub(crate) async fn provisioning_scope_reuses_existing_account_node_shunt_slot_when_group_is_full()
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
pub(crate) async fn provisioning_scope_claims_free_node_for_existing_account_without_assigned_slot()
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

use super::*;
