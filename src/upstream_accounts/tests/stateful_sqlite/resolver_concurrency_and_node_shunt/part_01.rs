use super::*;
use crate::tests::insert_test_pool_oauth_account;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};

async fn secondary_node_shunt_proxy_key(state: &Arc<AppState>) -> String {
    let mut manager = state.forward_proxy.lock().await;
    manager.apply_settings(ForwardProxySettings {
        proxy_urls: vec!["http://127.0.0.1:18080".to_string()],
        ..Default::default()
    });
    manager
        .binding_nodes()
        .into_iter()
        .find(|node| node.key != FORWARD_PROXY_DIRECT_KEY)
        .map(|node| node.key)
        .expect("secondary proxy binding key")
}

async fn insert_node_shunt_oauth_account(
    state: &Arc<AppState>,
    display_name: &str,
    email: &str,
    organization_id: &str,
    user_id: &str,
    group_name: &str,
) -> i64 {
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        display_name,
        email,
        organization_id,
        user_id,
    )
    .await;
    set_test_account_group_name(&state.pool, account_id, Some(group_name)).await;
    account_id
}

async fn save_node_shunt_group(
    state: &Arc<AppState>,
    group_name: &str,
    bound_proxy_keys: Vec<String>,
) {
    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        group_name,
        UpstreamAccountGroupMetadata {
            bound_proxy_keys,
            node_shunt_enabled: true,
            ..UpstreamAccountGroupMetadata::default()
        },
    )
    .await
    .expect("save node shunt metadata");
}

async fn disable_node_shunt_account(state: &Arc<AppState>, account_id: i64) {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query("UPDATE pool_upstream_accounts SET enabled = 0, updated_at = ?2 WHERE id = ?1")
        .bind(account_id)
        .bind(now_iso)
        .execute(&state.pool)
        .await
        .expect("disable provisioning account");
}

fn assert_in_flight_reservation_assignments(
    assignments: &UpstreamAccountNodeShuntAssignments,
    reserved_account_id: i64,
    overflow_account_id: i64,
    secondary_proxy_key: &str,
) {
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

async fn assert_refresh_failure_reassignment(
    state: &Arc<AppState>,
    failing_account_id: i64,
    fallback_account_id: i64,
    token_requests: &AtomicUsize,
) {
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
}

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
    let secondary_proxy_key = secondary_node_shunt_proxy_key(&state).await;
    let available_account_id = insert_node_shunt_oauth_account(
        &state,
        "Available Slot Account",
        "available-slot@example.com",
        "org_available_slot",
        "user_available_slot",
        "node-shunt-priority",
    )
    .await;
    let reserved_account_id = insert_node_shunt_oauth_account(
        &state,
        "Reserved Slot Account",
        "reserved-slot@example.com",
        "org_reserved_slot",
        "user_reserved_slot",
        "node-shunt-priority",
    )
    .await;
    let overflow_account_id = insert_node_shunt_oauth_account(
        &state,
        "Overflow Slot Account",
        "overflow-slot@example.com",
        "org_overflow_slot",
        "user_overflow_slot",
        "node-shunt-priority",
    )
    .await;
    save_node_shunt_group(
        &state,
        "node-shunt-priority",
        vec![
            FORWARD_PROXY_DIRECT_KEY.to_string(),
            secondary_proxy_key.clone(),
        ],
    )
    .await;

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
                model: None,
                proxy_key: Some(secondary_proxy_key.clone()),
                created_at: Instant::now(),
            },
        );
    set_test_account_group_name(&state.pool, available_account_id, None).await;

    let assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
        .await
        .expect("build node shunt assignments");

    assert_in_flight_reservation_assignments(
        &assignments,
        reserved_account_id,
        overflow_account_id,
        &secondary_proxy_key,
    );
}

#[tokio::test]
pub(crate) async fn node_shunt_assignments_keep_all_reserved_proxy_keys_occupied_for_one_account() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let secondary_proxy_key = secondary_node_shunt_proxy_key(&state).await;
    let reserved_account_id = insert_node_shunt_oauth_account(
        &state,
        "Reserved Multi Slot Account",
        "reserved-multi-slot@example.com",
        "org_reserved_multi_slot",
        "user_reserved_multi_slot",
        "node-shunt-multi-reserved",
    )
    .await;
    let overflow_account_id = insert_node_shunt_oauth_account(
        &state,
        "Overflow Multi Slot Account",
        "overflow-multi-slot@example.com",
        "org_overflow_multi_slot",
        "user_overflow_multi_slot",
        "node-shunt-multi-reserved",
    )
    .await;
    save_node_shunt_group(
        &state,
        "node-shunt-multi-reserved",
        vec![
            FORWARD_PROXY_DIRECT_KEY.to_string(),
            secondary_proxy_key.clone(),
        ],
    )
    .await;

    {
        let mut reservations = state
            .pool_routing_reservations
            .lock()
            .expect("pool routing reservations mutex poisoned");
        reservations.insert(
            "test-node-shunt-reservation-direct".to_string(),
            PoolRoutingReservation {
                account_id: reserved_account_id,
                model: None,
                proxy_key: Some(FORWARD_PROXY_DIRECT_KEY.to_string()),
                created_at: Instant::now(),
            },
        );
        reservations.insert(
            "test-node-shunt-reservation-secondary".to_string(),
            PoolRoutingReservation {
                account_id: reserved_account_id,
                model: None,
                proxy_key: Some(secondary_proxy_key.clone()),
                created_at: Instant::now(),
            },
        );
    }

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
    let secondary_proxy_key = secondary_node_shunt_proxy_key(&state).await;
    let account_id = insert_node_shunt_oauth_account(
        &state,
        "Globally Reserved Proxy Account",
        "globally-reserved@example.com",
        "org_globally_reserved",
        "user_globally_reserved",
        "node-shunt-global-reservation",
    )
    .await;
    let overflow_account_id = insert_node_shunt_oauth_account(
        &state,
        "Overflow Globally Reserved Proxy Account",
        "overflow-globally-reserved@example.com",
        "org_overflow_globally_reserved",
        "user_overflow_globally_reserved",
        "node-shunt-global-reservation",
    )
    .await;
    save_node_shunt_group(
        &state,
        "node-shunt-global-reservation",
        vec![
            FORWARD_PROXY_DIRECT_KEY.to_string(),
            secondary_proxy_key.clone(),
        ],
    )
    .await;

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

#[tokio::test]
pub(crate) async fn provisioning_scope_skips_proxy_keys_assigned_to_other_groups() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let secondary_proxy_key = secondary_node_shunt_proxy_key(&state).await;
    let account_id = insert_node_shunt_oauth_account(
        &state,
        "Provisioning Cross Group Account",
        "provision-cross-group@example.com",
        "org_provision_cross_group",
        "user_provision_cross_group",
        "node-shunt-provisioning-a",
    )
    .await;
    let occupying_account_id = insert_node_shunt_oauth_account(
        &state,
        "Occupying Cross Group Account",
        "occupying-cross-group@example.com",
        "org_occupying_cross_group",
        "user_occupying_cross_group",
        "node-shunt-provisioning-b",
    )
    .await;

    disable_node_shunt_account(&state, account_id).await;

    save_node_shunt_group(
        &state,
        "node-shunt-provisioning-a",
        vec![
            FORWARD_PROXY_DIRECT_KEY.to_string(),
            secondary_proxy_key.clone(),
        ],
    )
    .await;
    save_node_shunt_group(
        &state,
        "node-shunt-provisioning-b",
        vec![FORWARD_PROXY_DIRECT_KEY.to_string()],
    )
    .await;

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
pub(crate) async fn provisioning_scope_skips_proxy_keys_reserved_by_other_accounts() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let secondary_proxy_key = secondary_node_shunt_proxy_key(&state).await;
    let account_id = insert_node_shunt_oauth_account(
        &state,
        "Provisioning Reserved Proxy Account",
        "provision-reserved@example.com",
        "org_provision_reserved",
        "user_provision_reserved",
        "node-shunt-provisioning",
    )
    .await;
    disable_node_shunt_account(&state, account_id).await;

    save_node_shunt_group(
        &state,
        "node-shunt-provisioning",
        vec![
            FORWARD_PROXY_DIRECT_KEY.to_string(),
            secondary_proxy_key.clone(),
        ],
    )
    .await;

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
pub(crate) async fn provisioning_scope_reuses_live_reserved_proxy_key_for_same_account() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let secondary_proxy_key = secondary_node_shunt_proxy_key(&state).await;
    let account_id = insert_node_shunt_oauth_account(
        &state,
        "Provisioning Reserved Self Account",
        "provision-reserved-self@example.com",
        "org_provision_reserved_self",
        "user_provision_reserved_self",
        "node-shunt-provisioning",
    )
    .await;
    disable_node_shunt_account(&state, account_id).await;
    let bound_proxy_keys = vec![
        FORWARD_PROXY_DIRECT_KEY.to_string(),
        secondary_proxy_key.clone(),
    ];

    save_node_shunt_group(&state, "node-shunt-provisioning", bound_proxy_keys.clone()).await;

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
pub(crate) async fn provisioning_scope_rejects_existing_account_without_node_shunt_slot_when_group_is_full()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_node_shunt_oauth_account(
        &state,
        "Provisioned Disabled Account",
        "provision-disabled@example.com",
        "org_provision_disabled",
        "user_provision_disabled",
        "node-shunt-provisioning",
    )
    .await;
    let occupying_account_id = insert_node_shunt_oauth_account(
        &state,
        "Provisioned Occupying Account",
        "provision-occupying@example.com",
        "org_provision_occupying",
        "user_provision_occupying",
        "node-shunt-provisioning",
    )
    .await;
    save_node_shunt_group(
        &state,
        "node-shunt-provisioning",
        test_required_group_bound_proxy_keys(),
    )
    .await;
    disable_node_shunt_account(&state, account_id).await;

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

    assert_refresh_failure_reassignment(
        &state,
        failing_account_id,
        fallback_account_id,
        &token_requests,
    )
    .await;
    server.abort();
}
