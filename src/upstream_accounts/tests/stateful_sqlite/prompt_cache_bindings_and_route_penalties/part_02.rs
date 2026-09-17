use super::*;
use crate::tests::insert_test_pool_oauth_account;
use axum::http::header;

#[tokio::test]
pub(crate) async fn resolver_returns_specific_group_proxy_error_when_only_bad_groups_remain() {
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
pub(crate) async fn resolver_returns_specific_group_proxy_error_when_only_ungrouped_accounts_remain()
 {
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
pub(crate) async fn resolver_prefers_group_proxy_error_over_rate_limited_pool_when_no_healthy_candidates_remain()
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
pub(crate) async fn resolver_prefers_real_group_proxy_error_over_excluded_route_blockers() {
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
pub(crate) async fn resolver_treats_excluded_rate_limited_routes_as_unavailable() {
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
pub(crate) async fn resolver_prefers_group_proxy_error_over_excluded_route_cut_in_rejects() {
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
pub(crate) async fn resolver_prefers_group_proxy_error_over_degraded_pool_when_no_healthy_candidates_remain()
 {
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
pub(crate) async fn resolver_summarizes_multiple_group_proxy_errors_when_only_bad_groups_remain() {
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
pub(crate) async fn resolver_can_cut_out_from_group_proxy_blocked_sticky_account_when_allowed() {
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
pub(crate) async fn resolver_can_cut_out_from_ungrouped_sticky_account_when_allowed() {
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
pub(crate) async fn resolver_returns_group_proxy_error_for_sticky_account_when_cut_out_is_forbidden()
 {
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
pub(crate) async fn resolver_returns_ungrouped_error_for_sticky_account_when_cut_out_is_forbidden()
{
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
pub(crate) async fn resolver_preserves_sticky_account_when_cut_out_is_forbidden_by_tag_policy() {
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
pub(crate) async fn resolver_keeps_node_shunt_unassigned_fresh_candidate_assignable_for_live_routing()
 {
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
pub(crate) async fn resolver_prefers_sticky_cut_in_policy_over_group_proxy_error() {
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

#[tokio::test]
pub(crate) async fn update_oauth_login_session_preserves_pending_url_and_persists_metadata() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    insert_test_oauth_mailbox_session(
        &state.pool,
        "mailbox-session-1",
        "pending-sync@mail-tw.707079.xyz",
        OAUTH_MAILBOX_SOURCE_ATTACHED,
    )
    .await;

    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Original Pending".to_string()),
            email: None,
            group_name: Some("alpha".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: Some("before".to_string()),
            group_note: Some("alpha note".to_string()),
            concurrency_limit: None,
            account_id: None,
            tag_ids: vec![],
            is_mother: Some(false),
            mailbox_session_id: None,
            mailbox_address: None,
        }),
    )
    .await
    .expect("create oauth login session")
    .0;

    let updated = update_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("Updated Pending".to_string()),
            email: OptionalField::Missing,
            group_name: OptionalField::Value("beta".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            group_single_account_rotation_enabled: OptionalField::Missing,
            note: OptionalField::Value("after".to_string()),
            group_note: OptionalField::Value("beta shared".to_string()),
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![]),
            is_mother: OptionalField::Value(true),
            mailbox_session_id: OptionalField::Value("mailbox-session-1".to_string()),
            mailbox_address: OptionalField::Value("pending-sync@mail-tw.707079.xyz".to_string()),
        }),
    )
    .await
    .expect("update oauth login session")
    .0;

    assert_eq!(updated.login_id, created.login_id);
    assert_eq!(updated.auth_url, created.auth_url);
    assert_eq!(updated.redirect_uri, created.redirect_uri);
    assert_eq!(updated.expires_at, created.expires_at);

    let stored = load_login_session_by_login_id(&state.pool, &updated.login_id)
        .await
        .expect("load stored login session")
        .expect("stored login session should exist");
    assert_eq!(stored.display_name.as_deref(), Some("Updated Pending"));
    assert_eq!(stored.group_name.as_deref(), Some("beta"));
    assert_eq!(stored.note.as_deref(), Some("after"));
    assert_eq!(stored.group_note.as_deref(), Some("beta shared"));
    assert_eq!(stored.is_mother, 1);
    assert_eq!(
        parse_tag_ids_json(stored.tag_ids_json.as_deref()),
        Vec::<i64>::new()
    );
    assert_eq!(
        stored.mailbox_session_id.as_deref(),
        Some("mailbox-session-1")
    );
    assert_eq!(
        stored.mailbox_address.as_deref(),
        Some("pending-sync@mail-tw.707079.xyz")
    );
}

#[tokio::test]
pub(crate) async fn update_oauth_login_session_ignores_stale_baseline_updates() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Ordered Pending".to_string()),
            email: None,
            group_name: Some("alpha".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: Some("before".to_string()),
            group_note: Some("alpha note".to_string()),
            concurrency_limit: None,
            account_id: None,
            tag_ids: vec![],
            is_mother: Some(false),
            mailbox_session_id: None,
            mailbox_address: None,
        }),
    )
    .await
    .expect("create oauth login session")
    .0;

    let mut newer_headers = HeaderMap::new();
    newer_headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(&created.updated_at).expect("valid updated_at header"),
    );
    let newer = update_oauth_login_session(
        State(state.clone()),
        newer_headers,
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("Newest Pending".to_string()),
            email: OptionalField::Missing,
            group_name: OptionalField::Value("beta".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            group_single_account_rotation_enabled: OptionalField::Missing,
            note: OptionalField::Value("newest note".to_string()),
            group_note: OptionalField::Value("beta note".to_string()),
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![]),
            is_mother: OptionalField::Value(true),
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect("apply newer oauth login session update")
    .0;
    assert_ne!(newer.updated_at, created.updated_at);
    let newer_updated_at = newer.updated_at.clone();

    let mut stale_headers = HeaderMap::new();
    stale_headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(&created.updated_at).expect("valid updated_at header"),
    );
    let stale = update_oauth_login_session(
        State(state.clone()),
        stale_headers,
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("Stale Pending".to_string()),
            email: OptionalField::Missing,
            group_name: OptionalField::Value("gamma".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            group_single_account_rotation_enabled: OptionalField::Missing,
            note: OptionalField::Value("stale note".to_string()),
            group_note: OptionalField::Value("gamma note".to_string()),
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![]),
            is_mother: OptionalField::Value(false),
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect("stale oauth login session update should be ignored")
    .0;

    assert_eq!(stale.login_id, created.login_id);
    assert_eq!(stale.updated_at, newer_updated_at);

    let stored = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("load stored login session")
        .expect("stored login session should exist");
    assert_eq!(stored.display_name.as_deref(), Some("Newest Pending"));
    assert_eq!(stored.group_name.as_deref(), Some("beta"));
    assert_eq!(stored.note.as_deref(), Some("newest note"));
    assert_eq!(stored.group_note.as_deref(), Some("beta note"));
    assert_eq!(stored.is_mother, 1);
    assert_eq!(stored.updated_at, newer_updated_at);
}

#[tokio::test]
pub(crate) async fn update_oauth_login_session_preserves_omitted_fields() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    insert_test_oauth_mailbox_session(
        &state.pool,
        "mailbox-session-partial",
        "partial-sync@mail-tw.707079.xyz",
        OAUTH_MAILBOX_SOURCE_ATTACHED,
    )
    .await;

    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Keep Me".to_string()),
            email: None,
            group_name: Some("partial-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: Some("before partial patch".to_string()),
            group_note: Some("partial draft note".to_string()),
            concurrency_limit: None,
            account_id: None,
            tag_ids: vec![],
            is_mother: Some(true),
            mailbox_session_id: Some("mailbox-session-partial".to_string()),
            mailbox_address: Some("partial-sync@mail-tw.707079.xyz".to_string()),
        }),
    )
    .await
    .expect("create oauth login session")
    .0;

    let updated = update_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Missing,
            email: OptionalField::Missing,
            group_name: OptionalField::Missing,
            group_bound_proxy_keys: OptionalField::Missing,
            group_node_shunt_enabled: OptionalField::Missing,
            group_single_account_rotation_enabled: OptionalField::Missing,
            note: OptionalField::Value("after partial patch".to_string()),
            group_note: OptionalField::Missing,
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Missing,
            is_mother: OptionalField::Missing,
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect("update oauth login session")
    .0;

    assert_eq!(updated.login_id, created.login_id);
    assert_eq!(updated.auth_url, created.auth_url);
    assert_eq!(updated.redirect_uri, created.redirect_uri);
    assert_eq!(updated.expires_at, created.expires_at);

    let stored = load_login_session_by_login_id(&state.pool, &updated.login_id)
        .await
        .expect("load stored login session")
        .expect("stored login session should exist");
    assert_eq!(stored.display_name.as_deref(), Some("Keep Me"));
    assert_eq!(stored.group_name.as_deref(), Some("partial-group"));
    assert_eq!(stored.note.as_deref(), Some("after partial patch"));
    assert_eq!(stored.group_note.as_deref(), Some("partial draft note"));
    assert_eq!(stored.is_mother, 1);
    assert_eq!(
        parse_tag_ids_json(stored.tag_ids_json.as_deref()),
        Vec::<i64>::new()
    );
    assert_eq!(
        stored.mailbox_session_id.as_deref(),
        Some("mailbox-session-partial")
    );
    assert_eq!(
        stored.mailbox_address.as_deref(),
        Some("partial-sync@mail-tw.707079.xyz")
    );
}

#[tokio::test]
pub(crate) async fn update_oauth_login_session_clears_omitted_group_note_when_group_changes() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Move Draft Group".to_string()),
            email: None,
            group_name: Some("before-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: Some("before note".to_string()),
            group_note: Some("before draft note".to_string()),
            concurrency_limit: None,
            account_id: None,
            tag_ids: vec![],
            is_mother: Some(false),
            mailbox_session_id: None,
            mailbox_address: None,
        }),
    )
    .await
    .expect("create oauth login session")
    .0;

    let updated = update_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Missing,
            email: OptionalField::Missing,
            group_name: OptionalField::Value("after-group".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            group_single_account_rotation_enabled: OptionalField::Missing,
            note: OptionalField::Missing,
            group_note: OptionalField::Missing,
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Missing,
            is_mother: OptionalField::Missing,
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect("update oauth login session")
    .0;

    assert_eq!(updated.login_id, created.login_id);
    assert_eq!(updated.auth_url, created.auth_url);
    assert_eq!(updated.redirect_uri, created.redirect_uri);
    assert_eq!(updated.expires_at, created.expires_at);

    let stored = load_login_session_by_login_id(&state.pool, &updated.login_id)
        .await
        .expect("load stored login session")
        .expect("stored login session should exist");
    assert_eq!(stored.group_name.as_deref(), Some("after-group"));
    assert_eq!(stored.group_note, None);
    assert_eq!(stored.note.as_deref(), Some("before note"));
}

#[tokio::test]
pub(crate) async fn update_oauth_login_session_normalizes_blank_group_to_default_group() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Clear Group Note".to_string()),
            email: None,
            group_name: Some("draft-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: Some("before clearing group".to_string()),
            group_note: Some("draft group note".to_string()),
            concurrency_limit: None,
            account_id: None,
            tag_ids: vec![],
            is_mother: Some(false),
            mailbox_session_id: None,
            mailbox_address: None,
        }),
    )
    .await
    .expect("create oauth login session")
    .0;

    let _updated = update_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Missing,
            email: OptionalField::Missing,
            group_name: OptionalField::Value(String::new()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            group_single_account_rotation_enabled: OptionalField::Missing,
            note: OptionalField::Missing,
            group_note: OptionalField::Missing,
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Missing,
            is_mother: OptionalField::Missing,
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect("blank group should normalize to default group")
    .0;

    let stored = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("load stored login session")
        .expect("stored login session should exist");
    assert_eq!(
        stored.group_name.as_deref(),
        Some(DEFAULT_UPSTREAM_ACCOUNT_GROUP_NAME)
    );
    assert_eq!(stored.group_note, None);
    assert_eq!(stored.note.as_deref(), Some("before clearing group"));
}

#[tokio::test]
pub(crate) async fn updated_oauth_login_session_metadata_is_used_when_callback_persists_account() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    insert_test_oauth_mailbox_session(
        &state.pool,
        "mailbox-session-2",
        "callback-sync@mail-tw.707079.xyz",
        OAUTH_MAILBOX_SOURCE_ATTACHED,
    )
    .await;
    let created = create_callback_metadata_session(&state).await;
    update_callback_metadata_session(&state, &created.login_id).await;

    let updated_session = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("load updated session")
        .expect("updated session should exist");
    let account_id = persist_updated_callback_account(&state, updated_session).await;

    assert_updated_callback_account(&state, &created.login_id, account_id).await;
}

async fn create_callback_metadata_session(state: &Arc<AppState>) -> LoginSessionStatusResponse {
    create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Before Patch".to_string()),
            email: None,
            group_name: Some("old-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: Some("before note".to_string()),
            group_note: Some("old group note".to_string()),
            concurrency_limit: None,
            account_id: None,
            tag_ids: vec![],
            is_mother: Some(false),
            mailbox_session_id: None,
            mailbox_address: None,
        }),
    )
    .await
    .expect("create oauth login session")
    .0
}

async fn update_callback_metadata_session(state: &Arc<AppState>, login_id: &str) {
    let _ = update_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(login_id.to_string()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("After Patch".to_string()),
            email: OptionalField::Missing,
            group_name: OptionalField::Value("new-group".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            group_single_account_rotation_enabled: OptionalField::Missing,
            note: OptionalField::Value("after note".to_string()),
            group_note: OptionalField::Value("draft group note".to_string()),
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![]),
            is_mother: OptionalField::Value(true),
            mailbox_session_id: OptionalField::Value("mailbox-session-2".to_string()),
            mailbox_address: OptionalField::Value("callback-sync@mail-tw.707079.xyz".to_string()),
        }),
    )
    .await
    .expect("update oauth login session");
}

async fn persist_updated_callback_account(
    state: &Arc<AppState>,
    updated_session: OauthLoginSessionRow,
) -> i64 {
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let encrypted_credentials = encrypt_credentials(
        crypto_key,
        &StoredCredentials::Oauth(StoredOauthCredentials {
            access_token: "callback-access".to_string(),
            refresh_token: Some("callback-refresh".to_string()),
            id_token: test_id_token(
                "callback@example.com",
                Some("org_callback"),
                Some("user_callback"),
                Some("team"),
            ),
            token_type: Some("Bearer".to_string()),
        }),
    )
    .expect("encrypt oauth credentials");
    persist_oauth_callback_inner(
        state.as_ref(),
        PersistOauthCallbackInput {
            display_name: updated_session
                .display_name
                .clone()
                .expect("display name should be stored"),
            chosen_email: None,
            verified_email: None,
            session: updated_session,
            claims: test_claims(
                "callback@example.com",
                Some("org_callback"),
                Some("user_callback"),
            ),
            encrypted_credentials,
            has_refresh_token: true,
            token_expires_at: "2026-04-01T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("persist oauth callback")
}

async fn assert_updated_callback_account(state: &Arc<AppState>, login_id: &str, account_id: i64) {
    let account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth account row")
        .expect("oauth account should exist");
    assert_eq!(account.display_name, "After Patch");
    assert_eq!(account.group_name.as_deref(), Some("new-group"));
    assert_eq!(account.note.as_deref(), Some("after note"));
    assert_eq!(account.is_mother, 1);

    let account_tag_ids = sqlx::query_scalar::<_, i64>(
        r#"
            SELECT tag_id
            FROM pool_upstream_account_tags
            WHERE account_id = ?1
            ORDER BY tag_id ASC
            "#,
    )
    .bind(account_id)
    .fetch_all(&state.pool)
    .await
    .expect("load oauth account tags");
    assert!(account_tag_ids.is_empty());

    let group_note = sqlx::query_scalar::<_, Option<String>>(
        r#"
            SELECT note
            FROM pool_upstream_account_group_notes
            WHERE group_name = ?1
            "#,
    )
    .bind("new-group")
    .fetch_one(&state.pool)
    .await
    .expect("load group note");
    assert_eq!(group_note.as_deref(), Some("draft group note"));

    let completed_session = load_login_session_by_login_id(&state.pool, login_id)
        .await
        .expect("load completed session")
        .expect("completed session should exist");
    assert_eq!(completed_session.status, LOGIN_SESSION_STATUS_COMPLETED);
    assert_eq!(completed_session.account_id, Some(account_id));
}

#[tokio::test]
pub(crate) async fn persist_oauth_callback_preserves_group_node_shunt_for_legacy_pending_session() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let (created, bound_proxy_keys) = create_legacy_pending_session(&state).await;

    let account_id = persist_legacy_callback_account(&state, &created.login_id).await;

    let metadata = load_group_metadata(&state.pool, Some("legacy-group"))
        .await
        .expect("load group metadata");
    assert!(metadata.node_shunt_enabled);
    assert_eq!(metadata.bound_proxy_keys, bound_proxy_keys);

    let account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth account row")
        .expect("oauth account should exist");
    assert_eq!(account.group_name.as_deref(), Some("legacy-group"));

    let completed_session = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("load completed session")
        .expect("completed session should exist");
    assert_eq!(completed_session.status, LOGIN_SESSION_STATUS_COMPLETED);
    assert_eq!(completed_session.account_id, Some(account_id));
}

async fn create_legacy_pending_session(
    state: &Arc<AppState>,
) -> (LoginSessionStatusResponse, Vec<String>) {
    let bound_proxy_keys = test_required_group_bound_proxy_keys();
    let mut conn = state
        .pool
        .acquire()
        .await
        .expect("acquire group metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "legacy-group",
        UpstreamAccountGroupMetadata {
            bound_proxy_keys: bound_proxy_keys.clone(),
            node_shunt_enabled: true,
            ..UpstreamAccountGroupMetadata::default()
        },
    )
    .await
    .expect("seed legacy group metadata");
    drop(conn);
    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Legacy Pending".to_string()),
            email: None,
            group_name: Some("legacy-group".to_string()),
            group_bound_proxy_keys: Some(bound_proxy_keys.clone()),
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: Some("legacy note".to_string()),
            group_note: None,
            concurrency_limit: None,
            account_id: None,
            tag_ids: vec![],
            is_mother: Some(false),
            mailbox_session_id: None,
            mailbox_address: None,
        }),
    )
    .await
    .expect("create oauth login session")
    .0;
    sqlx::query(
        "UPDATE pool_oauth_login_sessions SET group_node_shunt_enabled = 0, group_node_shunt_enabled_requested = 0 WHERE login_id = ?1",
    )
    .bind(&created.login_id)
    .execute(&state.pool)
    .await
    .expect("downgrade pending session to legacy node shunt fields");
    (created, bound_proxy_keys)
}

async fn persist_legacy_callback_account(state: &Arc<AppState>, login_id: &str) -> i64 {
    let pending_session = load_login_session_by_login_id(&state.pool, login_id)
        .await
        .expect("load pending session")
        .expect("pending session should exist");
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let encrypted_credentials = encrypt_credentials(
        crypto_key,
        &StoredCredentials::Oauth(StoredOauthCredentials {
            access_token: "legacy-access".to_string(),
            refresh_token: Some("legacy-refresh".to_string()),
            id_token: test_id_token(
                "legacy@example.com",
                Some("org_legacy"),
                Some("user_legacy"),
                Some("team"),
            ),
            token_type: Some("Bearer".to_string()),
        }),
    )
    .expect("encrypt oauth credentials");
    persist_oauth_callback_inner(
        state.as_ref(),
        PersistOauthCallbackInput {
            display_name: pending_session
                .display_name
                .clone()
                .expect("display name should be stored"),
            chosen_email: None,
            verified_email: None,
            session: pending_session,
            claims: test_claims(
                "legacy@example.com",
                Some("org_legacy"),
                Some("user_legacy"),
            ),
            encrypted_credentials,
            has_refresh_token: true,
            token_expires_at: "2026-04-01T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("persist oauth callback")
}
