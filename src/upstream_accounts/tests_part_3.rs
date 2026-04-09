    #[tokio::test]
    async fn resolver_skips_account_when_effective_concurrency_limit_is_reached() {
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
    async fn node_shunt_assignments_preserve_slots_for_accounts_with_in_flight_reservations() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let secondary_proxy_key = {
            let mut manager = state.forward_proxy.lock().await;
            let mut settings = ForwardProxySettings::default();
            settings.proxy_urls = vec!["http://127.0.0.1:18080".to_string()];
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
                upstream_429_retry_enabled: false,
                upstream_429_max_retries: 0,
                concurrency_limit: 0,
            },
        )
        .await
        .expect("save node shunt metadata");
        drop(conn);

        let initial_assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
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
            Some(secondary_proxy_key.as_str())
        );

        state
            .pool_routing_reservations
            .lock()
            .expect("pool routing reservations mutex poisoned")
            .insert(
                "test-node-shunt-reservation".to_string(),
                PoolRoutingReservation {
                    account_id: reserved_account_id,
                    proxy_key: Some(secondary_proxy_key.clone()),
                    created_at: Instant::now(),
                },
            );
        set_test_account_group_name(&state.pool, available_account_id, None).await;

        let assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
            .await
            .expect("build node shunt assignments");

        assert_eq!(
            assignments
                .account_proxy_keys
                .get(&reserved_account_id)
                .map(String::as_str),
            Some(secondary_proxy_key.as_str())
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
    async fn node_shunt_assignments_keep_all_reserved_proxy_keys_occupied_for_one_account() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let secondary_proxy_key = {
            let mut manager = state.forward_proxy.lock().await;
            let mut settings = ForwardProxySettings::default();
            settings.proxy_urls = vec!["http://127.0.0.1:18080".to_string()];
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
                upstream_429_retry_enabled: false,
                upstream_429_max_retries: 0,
                concurrency_limit: 0,
            },
        )
        .await
        .expect("save node shunt metadata");
        drop(conn);

        let mut reservations = state
            .pool_routing_reservations
            .lock()
            .expect("pool routing reservations mutex poisoned");
        reservations.insert(
            "test-node-shunt-reservation-direct".to_string(),
            PoolRoutingReservation {
                account_id: reserved_account_id,
                proxy_key: Some(FORWARD_PROXY_DIRECT_KEY.to_string()),
                created_at: Instant::now(),
            },
        );
        reservations.insert(
            "test-node-shunt-reservation-secondary".to_string(),
            PoolRoutingReservation {
                account_id: reserved_account_id,
                proxy_key: Some(secondary_proxy_key.clone()),
                created_at: Instant::now(),
            },
        );
        drop(reservations);

        let assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
            .await
            .expect("build node shunt assignments");

        assert!(
            assignments
                .account_proxy_keys
                .get(&reserved_account_id)
                .is_some_and(|proxy_key| {
                    proxy_key == FORWARD_PROXY_DIRECT_KEY || proxy_key == &secondary_proxy_key
                })
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
                .map(|proxy_keys| proxy_keys.len()),
            Some(2)
        );
    }

    #[tokio::test]
    async fn node_shunt_assignments_prefer_primary_priority_before_fallback() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let fallback_account_id = insert_test_pool_api_key_account_with_options(
            &state,
            "Node Shunt Fallback",
            "sk-node-shunt-fallback",
            Some("node-shunt-priority"),
            Some("https://node-shunt-fallback.example.com/backend-api/codex"),
        )
        .await;
        let primary_account_id = insert_test_pool_api_key_account_with_options(
            &state,
            "Node Shunt Primary",
            "sk-node-shunt-primary",
            Some("node-shunt-priority"),
            Some("https://node-shunt-primary.example.com/backend-api/codex"),
        )
        .await;

        let mut fallback_rule = test_tag_routing_rule();
        fallback_rule.priority_tier = TagPriorityTier::Fallback;
        let fallback_tag = insert_tag(&state.pool, "node-shunt-fallback", &fallback_rule)
            .await
            .expect("insert fallback tag");
        let mut primary_rule = test_tag_routing_rule();
        primary_rule.priority_tier = TagPriorityTier::Primary;
        let primary_tag = insert_tag(&state.pool, "node-shunt-primary", &primary_rule)
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
    async fn node_shunt_assignments_keep_globally_reserved_proxy_keys_occupied() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let secondary_proxy_key = {
            let mut manager = state.forward_proxy.lock().await;
            let mut settings = ForwardProxySettings::default();
            settings.proxy_urls = vec!["http://127.0.0.1:18080".to_string()];
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
                    proxy_key: Some(FORWARD_PROXY_DIRECT_KEY.to_string()),
                    created_at: Instant::now(),
                },
            );

        let assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
            .await
            .expect("build node shunt assignments");

        assert_eq!(
            assignments
                .account_proxy_keys
                .get(&account_id)
                .map(String::as_str),
            Some(secondary_proxy_key.as_str())
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
                        && proxy_keys.contains(&secondary_proxy_key)
                })
        );
    }

    #[tokio::test]
    async fn node_shunt_assignments_keep_shared_proxy_keys_exclusive_across_groups() {
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

        set_test_account_group_name(&state.pool, group_a_account_id, Some("shared-node-group-a"))
            .await;
        set_test_account_group_name(&state.pool, group_b_account_id, Some("shared-node-group-b"))
            .await;

        let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
        for group_name in ["shared-node-group-a", "shared-node-group-b"] {
            save_group_metadata_record_conn(
                &mut conn,
                group_name,
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
    async fn node_shunt_sticky_reuse_preserves_slot_for_in_flight_account() {
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

        set_test_account_group_name(&state.pool, reserved_account_id, Some("node-shunt-sticky"))
            .await;
        set_test_account_group_name(&state.pool, available_account_id, Some("node-shunt-sticky"))
            .await;

        let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
        save_group_metadata_record_conn(
            &mut conn,
            "node-shunt-sticky",
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
            let mut settings = ForwardProxySettings::default();
            settings.proxy_urls = vec!["http://127.0.0.1:18080".to_string()];
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

        set_test_account_group_name(&state.pool, account_id, Some("node-shunt-provisioning-a"))
            .await;
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
            let mut settings = ForwardProxySettings::default();
            settings.proxy_urls = vec!["http://127.0.0.1:18080".to_string()];
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
            let mut settings = ForwardProxySettings::default();
            settings.proxy_urls = vec!["http://127.0.0.1:18080".to_string()];
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
    async fn provisioning_scope_rejects_existing_account_without_node_shunt_slot_when_group_is_full()
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

    #[tokio::test]
    async fn node_shunt_refresh_failure_reassigns_slot_within_same_request() {
        let (usage_base_url, oauth_issuer, token_requests, server) =
            spawn_token_failure_oauth_server(
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

        let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
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

        let assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
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
        server.abort();
    }

    #[tokio::test]
    async fn resolver_ignores_stale_sticky_routes_when_applying_concurrency_limit() {
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
    async fn resolver_allows_sticky_reuse_even_when_concurrency_limit_is_reached() {
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
    async fn resolver_keeps_sticky_reuse_even_when_higher_priority_accounts_exist() {
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
        let fallback_tag = insert_tag(&state.pool, "sticky-fallback-priority", &fallback_rule)
            .await
            .expect("insert sticky fallback tag");
        let mut primary_rule = test_tag_routing_rule();
        primary_rule.priority_tier = TagPriorityTier::Primary;
        let primary_tag = insert_tag(&state.pool, "sticky-primary-priority", &primary_rule)
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
            panic!("expected sticky reuse to keep the current account");
        };
        assert_eq!(account.account_id, sticky_account_id);
        assert_ne!(account.account_id, primary_account_id);
        assert_eq!(
            account.routing_source,
            PoolRoutingSelectionSource::StickyReuse
        );
    }

    #[tokio::test]
    async fn maintenance_pass_skips_secondary_overflow_accounts_until_secondary_interval() {
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
        timeout(Duration::from_secs(1), async {
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

    #[tokio::test]
    async fn account_actors_are_released_after_idle_commands_finish() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let account_id = insert_api_key_account(&state.pool, "Actor cleanup").await;

        assert_eq!(state.upstream_accounts.account_ops.actor_count(), 0);

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
                    note: Some("released".to_string()),
                    group_note: None,
                    concurrency_limit: None,
                    upstream_base_url: OptionalField::Missing,
                    enabled: None,
                    is_mother: None,
                    api_key: None,
                    local_primary_limit: None,
                    local_secondary_limit: None,
                    local_limit_unit: None,
                    tag_ids: None,
                },
            )
            .await
            .expect("update account");
        assert_eq!(state.upstream_accounts.account_ops.actor_count(), 0);

        state
            .upstream_accounts
            .account_ops
            .run_delete_account(state.clone(), account_id)
            .await
            .expect("delete account");
        assert_eq!(state.upstream_accounts.account_ops.actor_count(), 0);
    }

    #[tokio::test]
    async fn record_pool_route_http_failure_keeps_missing_scope_oauth_as_error() {
        let pool = test_pool().await;
        let account_id = insert_oauth_account(&pool, "Scope OAuth").await;

        record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            Some("sticky-scope"),
            StatusCode::UNAUTHORIZED,
            "pool upstream responded with 401: Missing scopes: api.responses.write",
            None,
        )
        .await
        .expect("record route failure");

        let status: String =
            sqlx::query_scalar("SELECT status FROM pool_upstream_accounts WHERE id = ?1")
                .bind(account_id)
                .fetch_one(&pool)
                .await
                .expect("load account status");
        assert_eq!(status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    }

    #[tokio::test]
    async fn record_pool_route_http_failure_marks_explicit_invalidated_oauth_for_reauth() {
        let pool = test_pool().await;
        let account_id = insert_oauth_account(&pool, "Invalidated OAuth").await;

        record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            Some("sticky-invalidated"),
            StatusCode::FORBIDDEN,
            "pool upstream responded with 403: Authentication token has been invalidated, please sign in again",
            None,
        )
        .await
        .expect("record route failure");

        let status: String =
            sqlx::query_scalar("SELECT status FROM pool_upstream_accounts WHERE id = ?1")
                .bind(account_id)
                .fetch_one(&pool)
                .await
                .expect("load account status");
        assert_eq!(status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
    }

    #[tokio::test]
    async fn record_pool_route_http_failure_keeps_bridge_exchange_oauth_as_error() {
        let pool = test_pool().await;
        let account_id = insert_oauth_account(&pool, "Bridge OAuth").await;

        record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            Some("sticky-bridge"),
            StatusCode::UNAUTHORIZED,
            "oauth bridge token exchange failed: oauth bridge responded with 502",
            None,
        )
        .await
        .expect("record route failure");

        let status: String =
            sqlx::query_scalar("SELECT status FROM pool_upstream_accounts WHERE id = ?1")
                .bind(account_id)
                .fetch_one(&pool)
                .await
                .expect("load account status");
        assert_eq!(status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    }

    #[tokio::test]
    async fn record_pool_route_http_failure_marks_402_as_hard_error_and_records_reason() {
        let pool = test_pool().await;
        let account_id = insert_api_key_account(&pool, "Plan Blocked Key").await;

        record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
            Some("sticky-402"),
            StatusCode::PAYMENT_REQUIRED,
            "pool upstream responded with 402: subscription required",
            Some("invk_402"),
        )
        .await
        .expect("record route failure");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load account row")
            .expect("account should exist");
        assert_eq!(row.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
        assert_eq!(
            row.last_action_reason_code.as_deref(),
            Some("upstream_http_402")
        );
        assert_eq!(
            row.last_action_source.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL)
        );
        assert_eq!(row.last_action_http_status, Some(402));
        assert_eq!(
            row.last_route_failure_kind.as_deref(),
            Some(PROXY_FAILURE_UPSTREAM_HTTP_402)
        );
        assert!(row.cooldown_until.is_none());
    }

    #[tokio::test]
    async fn route_triggered_402_summary_and_detail_export_as_upstream_rejected() {
        let pool = test_pool().await;
        let account_id = insert_oauth_account(&pool, "Workspace Blocked OAuth").await;

        record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            Some("sticky-402-workspace"),
            StatusCode::PAYMENT_REQUIRED,
            "initial usage snapshot attempt with configured user agent failed: usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}",
            Some("invk_workspace_402"),
        )
        .await
        .expect("record route-triggered 402 failure");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load route-triggered 402 row")
            .expect("route-triggered 402 row exists");
        let summary = build_summary_from_row(
            &row,
            None,
            row.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );

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
        assert_eq!(
            summary.last_action_reason_code.as_deref(),
            Some("upstream_http_402")
        );
        assert_eq!(
            summary.last_error.as_deref(),
            Some(
                "initial usage snapshot attempt with configured user agent failed: usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}"
            )
        );

        let detail = load_upstream_account_detail(&pool, account_id)
            .await
            .expect("load route-triggered 402 detail")
            .expect("route-triggered 402 detail exists");
        assert_eq!(
            detail.summary.display_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
        );
        assert_eq!(
            detail.summary.health_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
        );
        assert_eq!(
            detail.summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
        );
        assert_eq!(
            detail.summary.last_action_reason_code.as_deref(),
            Some("upstream_http_402")
        );
        assert_eq!(
            detail
                .recent_actions
                .first()
                .and_then(|event| event.reason_code.as_deref()),
            Some("upstream_http_402")
        );
        assert_eq!(
            detail
                .recent_actions
                .first()
                .and_then(|event| event.failure_kind.as_deref()),
            Some(PROXY_FAILURE_UPSTREAM_HTTP_402)
        );
    }

    #[tokio::test]
    async fn record_pool_route_http_failure_marks_quota_429_as_hard_error_and_records_reason() {
        let pool = test_pool().await;
        let account_id = insert_api_key_account(&pool, "Quota Exhausted Key").await;

        record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
            Some("sticky-429-quota"),
            StatusCode::TOO_MANY_REQUESTS,
            "insufficient_quota: pool upstream responded with 429: weekly cap exhausted",
            Some("invk_quota_429"),
        )
        .await
        .expect("record route failure");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load account row")
            .expect("account should exist");
        assert_eq!(row.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
        assert_eq!(
            row.last_action_reason_code.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
        );
        assert_eq!(row.last_action_http_status, Some(429));
        assert_eq!(
            row.last_route_failure_kind.as_deref(),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
        );
        assert!(row.cooldown_until.is_none());
    }

    #[tokio::test]
    async fn record_pool_route_http_failure_exports_first_plain_429_as_degraded_without_cooldown() {
        let pool = test_pool().await;
        let account_id = insert_api_key_account(&pool, "Degraded Plain 429").await;
        upsert_sticky_route(
            &pool,
            "sticky-degraded-first-hit",
            account_id,
            &format_utc_iso(Utc::now()),
        )
        .await
        .expect("seed sticky route");

        record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
            Some("sticky-degraded-first-hit"),
            StatusCode::TOO_MANY_REQUESTS,
            "pool upstream responded with 429: too many requests",
            Some("invk_degraded_first_hit"),
        )
        .await
        .expect("record first degraded 429 failure");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load degraded row")
            .expect("degraded row exists");
        assert_eq!(row.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(
            row.last_action.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_ROUTE_RETRYABLE_FAILURE)
        );
        assert_eq!(
            row.last_action_reason_code.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT)
        );
        assert_eq!(
            row.last_route_failure_kind.as_deref(),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
        );
        assert!(row.cooldown_until.is_none());
        assert_eq!(row.consecutive_route_failures, 1);
        assert!(row.temporary_route_failure_streak_started_at.is_some());
        assert_eq!(
            load_sticky_route(&pool, "sticky-degraded-first-hit")
                .await
                .expect("load sticky route after degraded hit")
                .map(|route| route.account_id),
            Some(account_id)
        );

        let summary = build_summary_from_row(
            &row,
            None,
            row.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );
        assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
        assert_eq!(summary.work_status, UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED);
    }

    #[tokio::test]
    async fn record_pool_route_http_failure_keeps_server_overloaded_as_retryable_without_cooldown()
    {
        let pool = test_pool().await;
        let account_id = insert_api_key_account(&pool, "Overloaded Key").await;

        record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
            Some("sticky-overloaded"),
            StatusCode::OK,
            "[upstream_response_failed] server_is_overloaded: Our servers are currently overloaded. Please try again later.",
            Some("invk_overloaded"),
        )
        .await
        .expect("record retryable overload route failure");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load overloaded row")
            .expect("overloaded row exists");
        assert_eq!(row.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(
            row.last_action.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_ROUTE_RETRYABLE_FAILURE)
        );
        assert_eq!(
            row.last_action_reason_code.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED)
        );
        assert_eq!(row.last_action_http_status, Some(200));
        assert_eq!(
            row.last_route_failure_kind.as_deref(),
            Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
        );
        assert!(row.cooldown_until.is_none());
        assert_eq!(row.consecutive_route_failures, 1);
        assert!(row.temporary_route_failure_streak_started_at.is_some());

        let summary = build_summary_from_row(
            &row,
            None,
            row.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );
        assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
        assert_eq!(summary.work_status, UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED);
    }

    #[tokio::test]
    async fn record_pool_route_transport_failure_starts_temporary_cooldown_after_streak_window_expires()
     {
        let pool = test_pool().await;
        let account_id = insert_api_key_account(&pool, "Cooldown Escalation").await;
        upsert_sticky_route(
            &pool,
            "sticky-degraded-cooldown",
            account_id,
            &format_utc_iso(Utc::now()),
        )
        .await
        .expect("seed sticky route");

        record_pool_route_transport_failure(
            &pool,
            account_id,
            Some("sticky-degraded-cooldown"),
            "failed to contact upstream",
            Some("invk_transport_first"),
        )
        .await
        .expect("record first transport failure");

        let stale_started_at = format_utc_iso(
            Utc::now()
                - ChronoDuration::seconds(POOL_ROUTE_TEMPORARY_FAILURE_DEGRADED_WINDOW_SECS + 1),
        );
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET temporary_route_failure_streak_started_at = ?2
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(&stale_started_at)
        .execute(&pool)
        .await
        .expect("stale degraded streak start");

        record_pool_route_transport_failure(
            &pool,
            account_id,
            Some("sticky-degraded-cooldown"),
            "failed to contact upstream again",
            Some("invk_transport_second"),
        )
        .await
        .expect("record second transport failure");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load escalated row")
            .expect("escalated row exists");
        assert_eq!(
            row.last_action.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_ROUTE_COOLDOWN_STARTED)
        );
        assert_eq!(
            row.last_route_failure_kind.as_deref(),
            Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM)
        );
        assert!(row.cooldown_until.is_some());
        assert_eq!(row.consecutive_route_failures, 2);
        assert_eq!(
            load_sticky_route(&pool, "sticky-degraded-cooldown")
                .await
                .expect("load sticky route after cooldown escalation")
                .map(|route| route.account_id),
            Some(account_id)
        );

        let summary = build_summary_from_row(
            &row,
            None,
            row.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );
        assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
        assert_eq!(summary.work_status, UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED);
    }

    #[tokio::test]
    async fn record_pool_route_transport_failure_caps_temporary_cooldown_at_sixty_seconds() {
        let pool = test_pool().await;
        let account_id = insert_api_key_account(&pool, "Cooldown Cap").await;
        let baseline_now = Utc::now();
        let baseline_now_iso = format_utc_iso(baseline_now);
        let stale_started_at = format_utc_iso(
            baseline_now
                - ChronoDuration::seconds(POOL_ROUTE_TEMPORARY_FAILURE_DEGRADED_WINDOW_SECS + 5),
        );
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET status = ?2,
                last_error = ?3,
                last_error_at = ?4,
                last_route_failure_at = ?4,
                last_route_failure_kind = ?5,
                cooldown_until = NULL,
                consecutive_route_failures = ?6,
                temporary_route_failure_streak_started_at = ?7,
                updated_at = ?4
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
        .bind("previous temporary failure")
        .bind(&baseline_now_iso)
        .bind(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM)
        .bind(7_i64)
        .bind(&stale_started_at)
        .execute(&pool)
        .await
        .expect("seed high temporary failure streak");

        record_pool_route_transport_failure(
            &pool,
            account_id,
            None,
            "failed to contact upstream again",
            Some("invk_transport_cap"),
        )
        .await
        .expect("record capped transport failure");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load capped row")
            .expect("capped row exists");
        let cooldown_until = row
            .cooldown_until
            .as_deref()
            .and_then(parse_rfc3339_utc)
            .expect("cooldown should be set");
        let route_failure_at = row
            .last_route_failure_at
            .as_deref()
            .and_then(parse_rfc3339_utc)
            .expect("route failure timestamp should be set");
        assert_eq!(
            cooldown_until - route_failure_at,
            ChronoDuration::seconds(POOL_ROUTE_TEMPORARY_FAILURE_COOLDOWN_MAX_SECS)
        );
    }

    #[test]
    fn classify_pool_account_http_failure_treats_usage_limit_reached_as_quota_exhausted() {
        let classification = classify_pool_account_http_failure(
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            StatusCode::TOO_MANY_REQUESTS,
            "pool upstream responded with 429: The usage limit has been reached",
        );

        assert_eq!(
            classification.disposition,
            UpstreamAccountFailureDisposition::HardUnavailable
        );
        assert_eq!(
            classification.reason_code,
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED
        );
        assert_eq!(
            classification.failure_kind,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED
        );
    }

    #[tokio::test]
    async fn quota_exhausted_oauth_summary_and_detail_export_as_rate_limited() {
        let pool = test_pool().await;
        let account_id = insert_oauth_account(&pool, "Quota Exhausted OAuth").await;

        record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            Some("sticky-quota-exhausted"),
            StatusCode::TOO_MANY_REQUESTS,
            "oauth_upstream_rejected_request: pool upstream responded with 429: The usage limit has been reached",
            Some("invk_quota_exhausted"),
        )
        .await
        .expect("record wrapped 429 route failure");

        let route_failure_row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load route failure row")
            .expect("route failure row exists");
        record_account_sync_recovery_blocked(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            &route_failure_row.status,
            UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED,
            "latest usage snapshot still shows an exhausted upstream usage limit window",
            route_failure_row.last_error.as_deref(),
            route_failure_row.last_route_failure_kind.as_deref(),
        )
        .await
        .expect("record blocked sync recovery");

        sqlx::query(
            r#"
            INSERT INTO pool_upstream_account_limit_samples (
                account_id, captured_at, limit_id, limit_name, plan_type,
                primary_used_percent, primary_window_minutes, primary_resets_at,
                secondary_used_percent, secondary_window_minutes, secondary_resets_at,
                credits_has_credits, credits_unlimited, credits_balance
            ) VALUES (
                ?1, ?2, NULL, NULL, 'team',
                100.0, 300, ?3,
                64.0, 10080, ?4,
                1, 0, '0.00'
            )
            "#,
        )
        .bind(account_id)
        .bind("2026-03-24T18:00:27Z")
        .bind("2026-03-30T16:06:33Z")
        .bind("2026-04-01T00:00:00Z")
        .execute(&pool)
        .await
        .expect("insert exhausted usage sample");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load updated row")
            .expect("updated row exists");
        let latest = load_latest_usage_sample(&pool, account_id)
            .await
            .expect("load latest usage sample");
        let summary = build_summary_from_row(
            &row,
            latest.as_ref(),
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
            summary.last_error.as_deref(),
            Some(
                "oauth_upstream_rejected_request: pool upstream responded with 429: The usage limit has been reached"
            )
        );
        assert_eq!(
            summary.last_action_reason_code.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)
        );
        assert_eq!(
            summary
                .primary_window
                .as_ref()
                .map(|window| window.used_percent),
            Some(100.0)
        );
        assert_eq!(
            summary
                .primary_window
                .as_ref()
                .and_then(|window| window.resets_at.as_deref()),
            Some("2026-03-30T16:06:33Z")
        );

        let detail = load_upstream_account_detail(&pool, account_id)
            .await
            .expect("load detail export")
            .expect("detail export exists");
        assert_eq!(
            detail.summary.display_status,
            UPSTREAM_ACCOUNT_STATUS_ACTIVE
        );
        assert_eq!(
            detail.summary.health_status,
            UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL
        );
        assert_eq!(
            detail.summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
        );
        assert_eq!(
            detail.summary.last_action_reason_code.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)
        );
        assert_eq!(
            detail
                .recent_actions
                .first()
                .map(|event| event.action.as_str()),
            Some(UPSTREAM_ACCOUNT_ACTION_SYNC_RECOVERY_BLOCKED)
        );
        assert_eq!(
            detail
                .recent_actions
                .first()
                .and_then(|event| event.reason_code.as_deref()),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)
        );
        assert_eq!(
            detail
                .recent_actions
                .first()
                .and_then(|event| event.failure_kind.as_deref()),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
        );
    }

    #[tokio::test]
    async fn sync_triggered_402_summary_and_detail_export_as_upstream_rejected() {
        let pool = test_pool().await;
        let account_id = insert_oauth_account(&pool, "Workspace Sync Blocked OAuth").await;

        record_account_sync_hard_unavailable(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            "upstream_http_402",
            "initial usage snapshot attempt with configured user agent failed: usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}",
            PROXY_FAILURE_UPSTREAM_HTTP_402,
        )
        .await
        .expect("record sync-triggered 402 hard unavailable");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load sync-triggered 402 row")
            .expect("sync-triggered 402 row exists");
        let summary = build_summary_from_row(
            &row,
            None,
            row.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );

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
        assert_eq!(
            summary.last_action.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_SYNC_HARD_UNAVAILABLE)
        );
        assert_eq!(
            summary.last_action_reason_code.as_deref(),
            Some("upstream_http_402")
        );
        assert_eq!(
            summary.last_error.as_deref(),
            Some(
                "initial usage snapshot attempt with configured user agent failed: usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}"
            )
        );

        let detail = load_upstream_account_detail(&pool, account_id)
            .await
            .expect("load sync-triggered 402 detail")
            .expect("sync-triggered 402 detail exists");
        assert_eq!(
            detail.summary.display_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
        );
        assert_eq!(
            detail.summary.health_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
        );
        assert_eq!(
            detail.summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
        );
        assert_eq!(
            detail.summary.last_action_reason_code.as_deref(),
            Some("upstream_http_402")
        );
        assert_eq!(
            detail
                .recent_actions
                .first()
                .map(|event| event.action.as_str()),
            Some(UPSTREAM_ACCOUNT_ACTION_SYNC_HARD_UNAVAILABLE)
        );
        assert_eq!(
            detail
                .recent_actions
                .first()
                .and_then(|event| event.reason_code.as_deref()),
            Some("upstream_http_402")
        );
        assert_eq!(
            detail
                .recent_actions
                .first()
                .and_then(|event| event.failure_kind.as_deref()),
            Some(PROXY_FAILURE_UPSTREAM_HTTP_402)
        );
    }

    #[tokio::test]
    async fn stale_quota_route_failure_does_not_hide_newer_sync_error() {
        let pool = test_pool().await;
        let account_id = insert_oauth_account(&pool, "Stale quota marker OAuth").await;

        record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            Some("sticky-stale-quota"),
            StatusCode::TOO_MANY_REQUESTS,
            "oauth_upstream_rejected_request: pool upstream responded with 429: The usage limit has been reached",
            Some("invk_stale_quota"),
        )
        .await
        .expect("record stale wrapped 429 route failure");

        record_account_sync_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            "usage snapshot parse error after refresh",
            UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR,
            None,
            PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
            None,
            false,
        )
        .await
        .expect("record newer sync failure");

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
        assert_eq!(
            summary.display_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER
        );
        assert_eq!(
            summary.health_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER
        );
        assert_eq!(
            summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
        );
        assert_eq!(
            summary.last_action_reason_code.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR)
        );
        assert_eq!(
            row.last_route_failure_kind.as_deref(),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
        );

        let detail = load_upstream_account_detail(&pool, account_id)
            .await
            .expect("load detail export")
            .expect("detail export exists");
        assert_eq!(
            detail.summary.display_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER
        );
        assert_eq!(
            detail.summary.health_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER
        );
        assert_eq!(
            detail.summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
        );
    }

    #[tokio::test]
    async fn stale_quota_route_failure_does_not_hide_newer_sync_402_error() {
        let pool = test_pool().await;
        let account_id = insert_oauth_account(&pool, "Stale quota marker 402 OAuth").await;

        record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            Some("sticky-stale-quota"),
            StatusCode::TOO_MANY_REQUESTS,
            "oauth_upstream_rejected_request: pool upstream responded with 429: The usage limit has been reached",
            Some("invk_stale_quota"),
        )
        .await
        .expect("record stale wrapped 429 route failure");

        record_account_sync_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            "initial usage snapshot attempt with configured user agent failed: usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}",
            "upstream_http_402",
            Some(StatusCode::PAYMENT_REQUIRED),
            PROXY_FAILURE_UPSTREAM_HTTP_402,
            None,
            false,
        )
        .await
        .expect("record legacy-style 402 sync failure");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load updated 402 row")
            .expect("updated 402 row exists");
        assert_eq!(
            row.last_route_failure_kind.as_deref(),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
        );

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
        assert_eq!(
            summary.last_action_reason_code.as_deref(),
            Some("upstream_http_402")
        );

        let detail = load_upstream_account_detail(&pool, account_id)
            .await
            .expect("load updated 402 detail")
            .expect("updated 402 detail exists");
        assert_eq!(
            detail.summary.display_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
        );
        assert_eq!(
            detail.summary.health_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
        );
        assert_eq!(
            detail.summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
        );
        assert_eq!(
            detail.summary.last_action_reason_code.as_deref(),
            Some("upstream_http_402")
        );
    }

    #[tokio::test]
    async fn blocked_api_key_manual_recovery_does_not_export_as_active_rate_limited() {
        let pool = test_pool().await;
        let account_id = insert_api_key_account(&pool, "Manual Recovery API Key").await;

        seed_hard_unavailable_route_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            Some(429),
        )
        .await;
        record_account_sync_recovery_blocked(
            &pool,
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

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load blocked api key row")
            .expect("blocked api key row exists");
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
        assert_eq!(
            summary.display_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER
        );
        assert_eq!(
            summary.health_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER
        );
        assert_eq!(
            summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
        );
        assert_eq!(
            summary.last_action_reason_code.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED)
        );

        let detail = load_upstream_account_detail(&pool, account_id)
            .await
            .expect("load blocked api key detail")
            .expect("blocked api key detail exists");
        assert_eq!(
            detail.summary.display_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER
        );
        assert_eq!(
            detail.summary.health_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER
        );
        assert_eq!(
            detail.summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
        );
        assert_eq!(
            detail.summary.last_action_reason_code.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED)
        );
        assert_eq!(
            detail
                .recent_actions
                .first()
                .and_then(|event| event.reason_code.as_deref()),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED)
        );
    }

    #[tokio::test]
    async fn explicit_reauth_phrase_without_reauth_reason_does_not_force_needs_reauth() {
        let pool = test_pool().await;
        let account_id = insert_api_key_account(&pool, "API key rejected wording").await;
        let now_iso = format_utc_iso(Utc::now());

        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET status = ?2,
                last_error = ?3,
                last_error_at = ?4,
                last_action = ?5,
                last_action_source = ?6,
                last_action_reason_code = ?7,
                last_action_reason_message = ?3,
                last_action_http_status = ?8,
                last_action_at = ?4,
                updated_at = ?4
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(UPSTREAM_ACCOUNT_STATUS_ERROR)
        .bind(
            "pool upstream responded with 403: Authentication token has been invalidated, please sign in again",
        )
        .bind(&now_iso)
        .bind(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
        .bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE)
        .bind("upstream_http_403")
        .bind(403)
        .execute(&pool)
        .await
        .expect("seed non-reauth rejection state");

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

        assert_ne!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
        assert_ne!(summary.health_status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
        assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
    }

    #[tokio::test]
    async fn legacy_oauth_explicit_reauth_error_without_reason_code_still_exports_needs_reauth() {
        let pool = test_pool().await;
        let account_id = insert_oauth_account(&pool, "Legacy OAuth Reauth").await;
        let now_iso = format_utc_iso(Utc::now());

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
                last_action_reason_code = NULL,
                last_action_reason_message = ?3,
                last_action_http_status = ?8,
                last_action_at = ?4,
                updated_at = ?4
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(UPSTREAM_ACCOUNT_STATUS_ERROR)
        .bind(
            "pool upstream responded with 403: Authentication token has been invalidated, please sign in again",
        )
        .bind(&now_iso)
        .bind(PROXY_FAILURE_UPSTREAM_HTTP_AUTH)
        .bind(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
        .bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE)
        .bind(403)
        .execute(&pool)
        .await
        .expect("seed legacy oauth reauth state");

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

        assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
        assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
        assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
    }

