use super::super::*;
use crate::tests::insert_test_pool_oauth_account;

#[tokio::test]
async fn resolver_returns_specific_group_proxy_error_when_only_bad_groups_remain() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Only Missing Binding",
        "only-missing-binding@example.com",
        "org_only_missing_binding",
        "user_only_missing_binding",
    )
    .await;
    set_test_account_group_name(&state.pool, account, Some("missing-bindings")).await;

    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve pool account");
    let PoolAccountResolution::BlockedByPolicy(message) = resolution else {
        panic!("expected specific group proxy error");
    };
    assert_eq!(
        message,
        "upstream account group \"missing-bindings\" has no bound forward proxy nodes; bind at least one proxy node to the group"
    );
}

#[tokio::test]
async fn resolver_returns_specific_group_proxy_error_when_only_ungrouped_accounts_remain() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Only Ungrouped Candidate",
        "only-ungrouped-candidate@example.com",
        "org_only_ungrouped_candidate",
        "user_only_ungrouped_candidate",
    )
    .await;
    set_test_account_group_name(&state.pool, account, None).await;

    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve pool account");
    let PoolAccountResolution::BlockedByPolicy(message) = resolution else {
        panic!("expected ungrouped account to surface a specific routing error");
    };
    assert_eq!(message, missing_account_group_error_message());
}

#[tokio::test]
async fn resolver_prefers_group_proxy_error_over_rate_limited_pool_when_no_healthy_candidates_remain()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let rate_limited = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Rate Limited Candidate",
        "rate-limited-candidate@example.com",
        "org_rate_limited_candidate",
        "user_rate_limited_candidate",
    )
    .await;
    let blocked = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Blocked Missing Binding",
        "blocked-missing-binding-mixed@example.com",
        "org_blocked_missing_binding_mixed",
        "user_blocked_missing_binding_mixed",
    )
    .await;
    set_test_account_group_name(&state.pool, blocked, Some("missing-bindings")).await;
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, rate_limited, &now_iso, Some(100.0), Some(50.0))
        .await;
    insert_limit_sample_with_usage(&state.pool, blocked, &now_iso, Some(1.0), Some(1.0)).await;

    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve pool account");
    let PoolAccountResolution::BlockedByPolicy(message) = resolution else {
        panic!("expected group proxy error to win over mixed rate-limited pool");
    };
    assert_eq!(
        message,
        "upstream account group \"missing-bindings\" has no bound forward proxy nodes; bind at least one proxy node to the group"
    );
}

#[tokio::test]
async fn resolver_prefers_real_group_proxy_error_over_excluded_route_blockers() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let excluded_blocked = insert_test_pool_oauth_account(
        &state,
        "Excluded Route Blocked",
        "oauth-excluded-route-blocked",
    )
    .await;
    let alternate_blocked = insert_test_pool_oauth_account(
        &state,
        "Alternate Route Blocked",
        "oauth-alternate-route-blocked",
    )
    .await;
    set_test_account_group_name(&state.pool, excluded_blocked, Some("same-route-missing")).await;
    set_test_account_group_name(&state.pool, alternate_blocked, Some("alternate-missing")).await;
    sqlx::query("UPDATE pool_upstream_accounts SET upstream_base_url = ?2 WHERE id = ?1")
        .bind(excluded_blocked)
        .bind("https://same-route.example.com/backend-api/codex")
        .execute(&state.pool)
        .await
        .expect("set excluded route upstream base");
    sqlx::query("UPDATE pool_upstream_accounts SET upstream_base_url = ?2 WHERE id = ?1")
        .bind(alternate_blocked)
        .bind("https://alternate-route.example.com/backend-api/codex")
        .execute(&state.pool)
        .await
        .expect("set alternate route upstream base");
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(
        &state.pool,
        excluded_blocked,
        &now_iso,
        Some(1.0),
        Some(1.0),
    )
    .await;
    insert_limit_sample_with_usage(
        &state.pool,
        alternate_blocked,
        &now_iso,
        Some(5.0),
        Some(1.0),
    )
    .await;
    let excluded_upstream_route_keys = HashSet::from([canonical_pool_upstream_route_key(
        &Url::parse("https://same-route.example.com/backend-api/codex")
            .expect("valid excluded route"),
    )]);

    let resolution =
        resolve_pool_account_for_request(&state, None, &[], &excluded_upstream_route_keys)
            .await
            .expect("resolve pool account");
    let PoolAccountResolution::BlockedByPolicy(message) = resolution else {
        panic!("expected actionable group proxy error to survive excluded same-route blockers");
    };
    assert!(message.contains("has no bound forward proxy nodes"));
}

#[tokio::test]
async fn resolver_treats_excluded_rate_limited_routes_as_unavailable() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let excluded_rate_limited = insert_test_pool_api_key_account_with_options(
        &state,
        "Excluded Rate Limited Route",
        "sk-excluded-rate-limited",
        None,
        Some("https://same-route-rate-limited.example.com/backend-api/codex"),
    )
    .await;
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(
        &state.pool,
        excluded_rate_limited,
        &now_iso,
        Some(100.0),
        Some(50.0),
    )
    .await;
    let excluded_upstream_route_keys = HashSet::from([canonical_pool_upstream_route_key(
        &Url::parse("https://same-route-rate-limited.example.com/backend-api/codex")
            .expect("valid excluded route"),
    )]);

    let resolution =
        resolve_pool_account_for_request(&state, None, &[], &excluded_upstream_route_keys)
            .await
            .expect("resolve pool account");
    assert!(matches!(resolution, PoolAccountResolution::Unavailable));
}

#[tokio::test]
async fn resolver_prefers_group_proxy_error_over_excluded_route_cut_in_rejects() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let sticky_source = insert_test_pool_api_key_account_with_options(
        &state,
        "Sticky Source Route",
        "sk-sticky-source-route",
        None,
        Some("https://route-a.example.com/backend-api/codex"),
    )
    .await;
    let excluded_cut_in_reject = insert_test_pool_api_key_account_with_options(
        &state,
        "Excluded Cut In Reject",
        "sk-excluded-cut-in-reject",
        None,
        Some("https://route-a.example.com/backend-api/codex"),
    )
    .await;
    let alternate_blocked = insert_test_pool_api_key_account_with_options(
        &state,
        "Alternate Blocked Route",
        "sk-alternate-blocked-route",
        None,
        Some("https://route-b.example.com/backend-api/codex"),
    )
    .await;
    set_test_account_group_name(&state.pool, alternate_blocked, Some("alternate-missing")).await;
    let no_cut_in_tag = insert_test_tag(
        &state.pool,
        "excluded-route-no-cut-in",
        &TagRoutingRule {
            allow_cut_out: true,
            allow_cut_in: false,
            priority_tier: TagPriorityTier::Normal,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            concurrency_limit: 0,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            available_models: vec![],
        },
    )
    .await
    .expect("insert no-cut-in tag");
    sync_account_tag_links(
        &state.pool,
        excluded_cut_in_reject,
        &[no_cut_in_tag.summary.id],
    )
    .await
    .expect("attach no-cut-in tag");
    let now_iso = format_utc_iso(Utc::now());
    upsert_sticky_route(
        &state.pool,
        "sticky-excluded-cut-in-reject",
        sticky_source,
        &now_iso,
    )
    .await
    .expect("upsert sticky route");
    insert_limit_sample_with_usage(&state.pool, sticky_source, &now_iso, Some(1.0), Some(1.0))
        .await;
    insert_limit_sample_with_usage(
        &state.pool,
        excluded_cut_in_reject,
        &now_iso,
        Some(5.0),
        Some(1.0),
    )
    .await;
    insert_limit_sample_with_usage(
        &state.pool,
        alternate_blocked,
        &now_iso,
        Some(10.0),
        Some(1.0),
    )
    .await;
    let excluded_upstream_route_keys = HashSet::from([canonical_pool_upstream_route_key(
        &Url::parse("https://route-a.example.com/backend-api/codex").expect("valid excluded route"),
    )]);

    let resolution = resolve_pool_account_for_request(
        &state,
        Some("sticky-excluded-cut-in-reject"),
        &[],
        &excluded_upstream_route_keys,
    )
    .await
    .expect("resolve pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("transit routing should ignore legacy group blockers, got {resolution:?}");
    };
    assert_eq!(account.account_id, alternate_blocked);
}

#[tokio::test]
async fn resolver_prefers_group_proxy_error_over_degraded_pool_when_no_healthy_candidates_remain() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let degraded = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Degraded Candidate",
        "degraded-candidate@example.com",
        "org_degraded_candidate",
        "user_degraded_candidate",
    )
    .await;
    let blocked = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Blocked Missing Binding",
        "blocked-missing-binding-degraded@example.com",
        "org_blocked_missing_binding_degraded",
        "user_blocked_missing_binding_degraded",
    )
    .await;
    set_test_account_group_name(&state.pool, blocked, Some("missing-bindings")).await;
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET status = ?1,
                last_error = ?2,
                last_error_at = ?3,
                last_route_failure_at = ?3,
                last_route_failure_kind = ?4,
                cooldown_until = NULL,
                consecutive_route_failures = 1,
                temporary_route_failure_streak_started_at = ?3,
                updated_at = ?3
            WHERE id = ?5
            "#,
    )
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind("test degraded plain 429")
    .bind(&now_iso)
    .bind(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    .bind(degraded)
    .execute(&state.pool)
    .await
    .expect("set degraded pool account state");
    insert_limit_sample_with_usage(&state.pool, blocked, &now_iso, Some(1.0), Some(1.0)).await;

    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve pool account");
    let PoolAccountResolution::BlockedByPolicy(message) = resolution else {
        panic!("expected group proxy error to win over degraded pool");
    };
    assert_eq!(
        message,
        "upstream account group \"missing-bindings\" has no bound forward proxy nodes; bind at least one proxy node to the group"
    );
}

#[tokio::test]
async fn resolver_summarizes_multiple_group_proxy_errors_when_only_bad_groups_remain() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let missing_binding = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Missing Binding Group",
        "missing-binding-group@example.com",
        "org_missing_binding_group",
        "user_missing_binding_group",
    )
    .await;
    let unselectable = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Unselectable Binding Group",
        "unselectable-binding-group@example.com",
        "org_unselectable_binding_group",
        "user_unselectable_binding_group",
    )
    .await;
    set_test_account_group_name(&state.pool, missing_binding, Some("group-a")).await;
    set_test_account_group_name(&state.pool, unselectable, Some("group-b")).await;
    upsert_test_group_binding(
        &state.pool,
        "group-b",
        vec!["unselectable-bound-node".to_string()],
    )
    .await;
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, missing_binding, &now_iso, Some(1.0), Some(1.0))
        .await;
    insert_limit_sample_with_usage(&state.pool, unselectable, &now_iso, Some(5.0), Some(1.0)).await;

    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve pool account");
    let PoolAccountResolution::BlockedByPolicy(message) = resolution else {
        panic!("expected summarized group proxy error");
    };
    assert!(message.contains(
            "upstream account group \"group-a\" has no bound forward proxy nodes; bind at least one proxy node to the group"
        ));
    assert!(
        message.contains("plus 1 additional upstream account group routing configuration issue(s)")
    );
}

#[tokio::test]
async fn resolver_can_cut_out_from_group_proxy_blocked_sticky_account_when_allowed() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let sticky_account = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Sticky Invalid Group",
        "sticky-invalid-group@example.com",
        "org_sticky_invalid_group",
        "user_sticky_invalid_group",
    )
    .await;
    let fallback_account = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Fallback Healthy Group",
        "fallback-healthy-group@example.com",
        "org_fallback_healthy_group",
        "user_fallback_healthy_group",
    )
    .await;
    set_test_account_group_name(&state.pool, sticky_account, Some("sticky-missing")).await;
    upsert_sticky_route(
        &state.pool,
        "sticky-group-proxy-blocked",
        sticky_account,
        &format_utc_iso(Utc::now()),
    )
    .await
    .expect("upsert sticky route");

    let resolution = resolve_pool_account_for_request(
        &state,
        Some("sticky-group-proxy-blocked"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected resolver to cut out from blocked sticky account");
    };
    assert_eq!(account.account_id, fallback_account);
}

#[tokio::test]
async fn resolver_can_cut_out_from_ungrouped_sticky_account_when_allowed() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let sticky_account = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Sticky Ungrouped Account",
        "sticky-ungrouped-account@example.com",
        "org_sticky_ungrouped_account",
        "user_sticky_ungrouped_account",
    )
    .await;
    let fallback_account = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Fallback Healthy Grouped Account",
        "fallback-healthy-grouped-account@example.com",
        "org_fallback_healthy_grouped_account",
        "user_fallback_healthy_grouped_account",
    )
    .await;
    set_test_account_group_name(&state.pool, sticky_account, None).await;
    upsert_sticky_route(
        &state.pool,
        "sticky-ungrouped-account",
        sticky_account,
        &format_utc_iso(Utc::now()),
    )
    .await
    .expect("upsert sticky route");

    let resolution = resolve_pool_account_for_request(
        &state,
        Some("sticky-ungrouped-account"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected resolver to cut out from ungrouped sticky account");
    };
    assert_eq!(account.account_id, fallback_account);
}

#[tokio::test]
async fn resolver_returns_group_proxy_error_for_sticky_account_when_cut_out_is_forbidden() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let sticky_account = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Sticky Invalid Locked Group",
        "sticky-invalid-locked-group@example.com",
        "org_sticky_invalid_locked_group",
        "user_sticky_invalid_locked_group",
    )
    .await;
    let _fallback_account = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Ignored Healthy Group",
        "ignored-healthy-group@example.com",
        "org_ignored_healthy_group",
        "user_ignored_healthy_group",
    )
    .await;
    set_test_account_group_name(&state.pool, sticky_account, Some("sticky-missing")).await;
    let lock_tag = insert_test_tag(
        &state.pool,
        "sticky-lock",
        &TagRoutingRule {
            allow_cut_out: false,
            allow_cut_in: true,
            priority_tier: TagPriorityTier::Normal,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            concurrency_limit: 0,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            available_models: vec![],
        },
    )
    .await
    .expect("insert lock tag");
    sync_account_tag_links(&state.pool, sticky_account, &[lock_tag.summary.id])
        .await
        .expect("attach lock tag");
    upsert_sticky_route(
        &state.pool,
        "sticky-group-proxy-locked",
        sticky_account,
        &format_utc_iso(Utc::now()),
    )
    .await
    .expect("upsert sticky route");

    let resolution = resolve_pool_account_for_request(
        &state,
        Some("sticky-group-proxy-locked"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve pool account");
    let PoolAccountResolution::AssignedBlocked(blocked) = resolution else {
        panic!("expected sticky group proxy block to preserve the assigned account");
    };
    assert_eq!(blocked.account.account_id, sticky_account);
    assert_eq!(
        blocked.failure_kind,
        PROXY_FAILURE_POOL_ASSIGNED_ACCOUNT_BLOCKED
    );
    assert_eq!(
        blocked.message,
        "upstream account group \"sticky-missing\" has no bound forward proxy nodes; bind at least one proxy node to the group"
    );
}

#[tokio::test]
async fn resolver_returns_ungrouped_error_for_sticky_account_when_cut_out_is_forbidden() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let sticky_account = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Sticky Ungrouped Locked Account",
        "sticky-ungrouped-locked-account@example.com",
        "org_sticky_ungrouped_locked_account",
        "user_sticky_ungrouped_locked_account",
    )
    .await;
    let _fallback_account = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Ignored Healthy Grouped Account",
        "ignored-healthy-grouped-account@example.com",
        "org_ignored_healthy_grouped_account",
        "user_ignored_healthy_grouped_account",
    )
    .await;
    set_test_account_group_name(&state.pool, sticky_account, None).await;
    let lock_tag = insert_test_tag(
        &state.pool,
        "sticky-ungrouped-lock",
        &TagRoutingRule {
            allow_cut_out: false,
            allow_cut_in: true,
            priority_tier: TagPriorityTier::Normal,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            concurrency_limit: 0,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            available_models: vec![],
        },
    )
    .await
    .expect("insert lock tag");
    sync_account_tag_links(&state.pool, sticky_account, &[lock_tag.summary.id])
        .await
        .expect("attach lock tag");
    upsert_sticky_route(
        &state.pool,
        "sticky-ungrouped-locked",
        sticky_account,
        &format_utc_iso(Utc::now()),
    )
    .await
    .expect("upsert sticky route");

    let resolution = resolve_pool_account_for_request(
        &state,
        Some("sticky-ungrouped-locked"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve pool account");
    let PoolAccountResolution::AssignedBlocked(blocked) = resolution else {
        panic!("expected sticky ungrouped block to preserve the assigned account");
    };
    assert_eq!(blocked.account.account_id, sticky_account);
    assert_eq!(
        blocked.failure_kind,
        PROXY_FAILURE_POOL_ASSIGNED_ACCOUNT_BLOCKED
    );
    assert_eq!(blocked.message, missing_account_group_error_message());
}

#[tokio::test]
async fn resolver_preserves_sticky_account_when_cut_out_is_forbidden_by_tag_policy() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let sticky_account = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Sticky Locked Account",
        "sticky-locked-account@example.com",
        "org_sticky_locked_account",
        "user_sticky_locked_account",
    )
    .await;
    let fallback_account = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Sticky Locked Fallback",
        "sticky-locked-fallback@example.com",
        "org_sticky_locked_fallback",
        "user_sticky_locked_fallback",
    )
    .await;
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET token_expires_at = ?2
            WHERE id = ?1
            "#,
    )
    .bind(sticky_account)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .expect("expire sticky account token");
    set_test_account_access_token(
        &state.pool,
        crypto_key,
        sticky_account,
        test_access_token_expiring_at(Utc::now() - ChronoDuration::minutes(5)),
    )
    .await;
    let lock_tag = insert_test_tag(
        &state.pool,
        "sticky-cut-out-forbidden",
        &TagRoutingRule {
            allow_cut_out: false,
            allow_cut_in: true,
            priority_tier: TagPriorityTier::Normal,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            concurrency_limit: 0,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            available_models: vec![],
        },
    )
    .await
    .expect("insert lock tag");
    sync_account_tag_links(&state.pool, sticky_account, &[lock_tag.summary.id])
        .await
        .expect("attach lock tag");
    upsert_sticky_route(
        &state.pool,
        "sticky-cut-out-forbidden-policy",
        sticky_account,
        &now_iso,
    )
    .await
    .expect("upsert sticky route");

    let resolution = resolve_pool_account_for_request(
        &state,
        Some("sticky-cut-out-forbidden-policy"),
        &[fallback_account],
        &HashSet::new(),
    )
    .await
    .expect("resolve pool account");
    let PoolAccountResolution::AssignedBlocked(blocked) = resolution else {
        panic!("expected sticky tag policy block to preserve the assigned account");
    };
    assert_eq!(blocked.account.account_id, sticky_account);
    assert_eq!(
        blocked.failure_kind,
        PROXY_FAILURE_POOL_ASSIGNED_ACCOUNT_BLOCKED
    );
    assert_eq!(
        blocked.message,
        "sticky conversation cannot cut out of the current account because routing policy forbids it"
    );
}

#[tokio::test]
async fn resolver_keeps_node_shunt_unassigned_fresh_candidate_assignable_for_live_routing() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let occupying_account = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Node Shunt Occupying Account",
        "node-shunt-occupying@example.com",
        "org_node_shunt_occupying",
        "user_node_shunt_occupying",
    )
    .await;
    let fallback_account = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Node Shunt Fallback Account",
        "node-shunt-fallback@example.com",
        "org_node_shunt_fallback",
        "user_node_shunt_fallback",
    )
    .await;
    set_test_account_group_name(&state.pool, occupying_account, Some("node-shunt-live")).await;
    set_test_account_group_name(&state.pool, fallback_account, Some("node-shunt-live")).await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-live",
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
    .expect("save node shunt live metadata");
    drop(conn);

    let resolution =
        resolve_pool_account_for_request(&state, None, &[occupying_account], &HashSet::new())
            .await
            .expect("resolve pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected live routing to keep the unassigned node shunt account assignable");
    };
    assert_eq!(account.account_id, fallback_account);
    let ForwardProxyRouteScope::BoundGroup {
        group_name,
        bound_proxy_keys,
    } = &account.forward_proxy_scope
    else {
        panic!("expected unassigned node shunt account to reuse a bound-group live scope");
    };
    assert_eq!(group_name, "node-shunt-live");
    assert_eq!(bound_proxy_keys, &test_required_group_bound_proxy_keys());
}

#[tokio::test]
async fn resolver_prefers_sticky_cut_in_policy_over_group_proxy_error() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let sticky_source = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Sticky Source",
        "sticky-source@example.com",
        "org_sticky_source",
        "user_sticky_source",
    )
    .await;
    let blocked_target = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Sticky Cut In Blocked",
        "sticky-cut-in-blocked@example.com",
        "org_sticky_cut_in_blocked",
        "user_sticky_cut_in_blocked",
    )
    .await;
    set_test_account_group_name(&state.pool, blocked_target, Some("sticky-cut-in-missing")).await;
    let no_cut_in_tag = insert_test_tag(
        &state.pool,
        "sticky-no-cut-in",
        &TagRoutingRule {
            allow_cut_out: true,
            allow_cut_in: false,
            priority_tier: TagPriorityTier::Normal,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            concurrency_limit: 0,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            available_models: vec![],
        },
    )
    .await
    .expect("insert no cut-in tag");
    sync_account_tag_links(&state.pool, blocked_target, &[no_cut_in_tag.summary.id])
        .await
        .expect("attach no cut-in tag");
    upsert_sticky_route(
        &state.pool,
        "sticky-cut-in-policy-first",
        sticky_source,
        &format_utc_iso(Utc::now()),
    )
    .await
    .expect("upsert sticky route");

    let resolution = resolve_pool_account_for_request(
        &state,
        Some("sticky-cut-in-policy-first"),
        &[sticky_source],
        &HashSet::new(),
    )
    .await
    .expect("resolve pool account");
    assert!(matches!(resolution, PoolAccountResolution::Unavailable));
}
