use super::*;

async fn seed_single_rotation_reset_accounts(
    state: &Arc<AppState>,
    group_name: &str,
    now_iso: &str,
) -> (i64, i64, i64) {
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let exhausted = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Rotation exhausted",
        "rotation-exhausted@example.com",
        "org_rotation_exhausted",
        "user_rotation_exhausted",
    )
    .await;
    let closer_reset = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Rotation closer reset",
        "rotation-closer-reset@example.com",
        "org_rotation_closer_reset",
        "user_rotation_closer_reset",
    )
    .await;
    let farther_reset = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Rotation farther reset",
        "rotation-farther-reset@example.com",
        "org_rotation_farther_reset",
        "user_rotation_farther_reset",
    )
    .await;
    for account_id in [exhausted, closer_reset, farther_reset] {
        set_test_account_group_name(&state.pool, account_id, Some(group_name)).await;
    }
    sqlx::query(
        r#"
            INSERT INTO pool_upstream_account_group_notes (
                group_name, note, single_account_rotation_enabled, created_at, updated_at
            ) VALUES (?1, '', 1, ?2, ?2)
            ON CONFLICT(group_name) DO UPDATE SET
                single_account_rotation_enabled = excluded.single_account_rotation_enabled,
                updated_at = excluded.updated_at
            "#,
    )
    .bind(group_name)
    .bind(now_iso)
    .execute(&state.pool)
    .await
    .expect("enable single-account rotation for group");
    upsert_test_group_binding(
        &state.pool,
        group_name,
        vec![FORWARD_PROXY_DIRECT_KEY.to_string()],
    )
    .await;
    (exhausted, closer_reset, farther_reset)
}

#[test]
pub(crate) fn reset_proximity_sorts_before_usage_pressure_after_priority_gates() {
    let closer_secondary_reset = PoolRoutingCandidateScore {
        eligibility: PoolRoutingCandidateEligibility::Assignable,
        route_binding_failure_penalty: 0,
        model_route_penalty: 0,
        routing_priority_rank: 1,
        capacity_lane: PoolRoutingCandidateCapacityLane::Primary,
        dispatch_state: PoolRoutingCandidateDispatchState::ReadyOnOwnedNode,
        single_account_rotation_enabled: true,
        secondary_reset_proximity_secs: Some(60),
        primary_reset_proximity_secs: Some(60 * 60 * 4),
        scarcity_score: 0.95,
        effective_load: 2,
        last_selected_at: Some("2026-03-23T12:00:00Z".to_string()),
        account_id: 20,
    };
    let farther_secondary_reset = PoolRoutingCandidateScore {
        eligibility: PoolRoutingCandidateEligibility::Assignable,
        route_binding_failure_penalty: 0,
        model_route_penalty: 0,
        routing_priority_rank: 1,
        capacity_lane: PoolRoutingCandidateCapacityLane::Primary,
        dispatch_state: PoolRoutingCandidateDispatchState::ReadyOnOwnedNode,
        single_account_rotation_enabled: true,
        secondary_reset_proximity_secs: Some(60 * 60 * 24),
        primary_reset_proximity_secs: Some(60),
        scarcity_score: 0.05,
        effective_load: 0,
        last_selected_at: None,
        account_id: 21,
    };

    assert_eq!(
        compare_pool_routing_candidate_scores(&closer_secondary_reset, &farther_secondary_reset),
        std::cmp::Ordering::Less,
        "7-day reset proximity should sort before short-window reset and usage pressure",
    );
}

#[test]
pub(crate) fn reset_proximity_places_missing_reset_after_known_reset() {
    let known_reset = PoolRoutingCandidateScore {
        eligibility: PoolRoutingCandidateEligibility::Assignable,
        route_binding_failure_penalty: 0,
        model_route_penalty: 0,
        routing_priority_rank: 1,
        capacity_lane: PoolRoutingCandidateCapacityLane::Primary,
        dispatch_state: PoolRoutingCandidateDispatchState::ReadyOnOwnedNode,
        single_account_rotation_enabled: true,
        secondary_reset_proximity_secs: Some(60 * 5),
        primary_reset_proximity_secs: None,
        scarcity_score: 0.9,
        effective_load: 3,
        last_selected_at: Some("2026-03-23T12:00:00Z".to_string()),
        account_id: 22,
    };
    let missing_reset = PoolRoutingCandidateScore {
        eligibility: PoolRoutingCandidateEligibility::Assignable,
        route_binding_failure_penalty: 0,
        model_route_penalty: 0,
        routing_priority_rank: 1,
        capacity_lane: PoolRoutingCandidateCapacityLane::Primary,
        dispatch_state: PoolRoutingCandidateDispatchState::ReadyOnOwnedNode,
        single_account_rotation_enabled: true,
        secondary_reset_proximity_secs: None,
        primary_reset_proximity_secs: Some(1),
        scarcity_score: 0.0,
        effective_load: 0,
        last_selected_at: None,
        account_id: 23,
    };

    assert_eq!(
        compare_pool_routing_candidate_scores(&known_reset, &missing_reset),
        std::cmp::Ordering::Less,
        "known 7-day reset times should beat accounts without a 7-day reset time",
    );
}

#[test]
pub(crate) fn reset_proximity_does_not_change_default_sort_when_rotation_is_disabled() {
    let closer_secondary_reset = PoolRoutingCandidateScore {
        eligibility: PoolRoutingCandidateEligibility::Assignable,
        route_binding_failure_penalty: 0,
        model_route_penalty: 0,
        routing_priority_rank: 1,
        capacity_lane: PoolRoutingCandidateCapacityLane::Primary,
        dispatch_state: PoolRoutingCandidateDispatchState::ReadyOnOwnedNode,
        single_account_rotation_enabled: false,
        secondary_reset_proximity_secs: Some(60),
        primary_reset_proximity_secs: Some(60),
        scarcity_score: 0.95,
        effective_load: 2,
        last_selected_at: Some("2026-03-23T12:00:00Z".to_string()),
        account_id: 24,
    };
    let lower_pressure = PoolRoutingCandidateScore {
        eligibility: PoolRoutingCandidateEligibility::Assignable,
        route_binding_failure_penalty: 0,
        model_route_penalty: 0,
        routing_priority_rank: 1,
        capacity_lane: PoolRoutingCandidateCapacityLane::Primary,
        dispatch_state: PoolRoutingCandidateDispatchState::ReadyOnOwnedNode,
        single_account_rotation_enabled: false,
        secondary_reset_proximity_secs: Some(60 * 60 * 24),
        primary_reset_proximity_secs: Some(60 * 60 * 24),
        scarcity_score: 0.05,
        effective_load: 0,
        last_selected_at: None,
        account_id: 25,
    };

    assert_eq!(
        compare_pool_routing_candidate_scores(&closer_secondary_reset, &lower_pressure),
        std::cmp::Ordering::Greater,
        "disabled groups should still fall through to usage pressure tie-breakers",
    );
}

#[tokio::test]
pub(crate) async fn resolver_uses_reset_time_candidate_after_single_account_rotation_429() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let group_name = "single-rotation-reset-order";
    let sticky_key = "sticky-single-rotation-reset-order";
    let now = Utc::now();
    let now_iso = format_utc_iso(now);
    let (exhausted, closer_reset, farther_reset) =
        seed_single_rotation_reset_accounts(&state, group_name, &now_iso).await;

    upsert_sticky_route(&state.pool, sticky_key, exhausted, &now_iso)
        .await
        .expect("seed sticky route");
    insert_limit_sample_with_reset_times(
        &state.pool,
        exhausted,
        &now_iso,
        Some(&format_utc_iso(now + ChronoDuration::minutes(5))),
        Some(&format_utc_iso(now + ChronoDuration::minutes(5))),
        1.0,
        1.0,
    )
    .await;
    insert_limit_sample_with_reset_times(
        &state.pool,
        closer_reset,
        &now_iso,
        Some(&format_utc_iso(now + ChronoDuration::hours(4))),
        Some(&format_utc_iso(now + ChronoDuration::minutes(30))),
        95.0,
        95.0,
    )
    .await;
    insert_limit_sample_with_reset_times(
        &state.pool,
        farther_reset,
        &now_iso,
        Some(&format_utc_iso(now + ChronoDuration::minutes(10))),
        Some(&format_utc_iso(now + ChronoDuration::days(2))),
        1.0,
        1.0,
    )
    .await;

    record_pool_route_http_failure(
        &state.pool,
        exhausted,
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        true,
        Some(sticky_key),
        StatusCode::TOO_MANY_REQUESTS,
        "pool upstream responded with 429: temporary rate limit",
        Some("invk_single_rotation_reset_order"),
    )
    .await
    .expect("record final 429");

    let sticky_after_failure: Option<i64> =
        sqlx::query_scalar("SELECT account_id FROM pool_sticky_routes WHERE sticky_key = ?1")
            .bind(sticky_key)
            .fetch_optional(&state.pool)
            .await
            .expect("load sticky route after 429");
    assert_eq!(sticky_after_failure, None);

    let resolution =
        resolve_pool_account_for_request(&state, Some(sticky_key), &[], &HashSet::new())
            .await
            .expect("resolve after final 429");
    let PoolAccountResolution::Resolved(account) = resolution.clone() else {
        panic!("expected resolver to select the next reset-time candidate, got {resolution:?}");
    };
    assert_eq!(account.account_id, closer_reset);
    assert!(account.single_account_rotation_enabled);
}

#[tokio::test]
pub(crate) async fn resolver_keeps_quota_exhausted_accounts_in_rate_limited_terminal_state_after_sync_block()
 {
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
            UPSTREAM_ACCOUNT_STATUS_ERROR,
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
pub(crate) async fn resolver_skips_candidate_when_group_has_no_bound_proxy_keys() {
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
pub(crate) async fn resolver_skips_candidate_when_group_has_only_unselectable_bound_proxies() {
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
pub(crate) async fn resolver_skips_ungrouped_candidate_when_healthy_grouped_account_exists() {
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
