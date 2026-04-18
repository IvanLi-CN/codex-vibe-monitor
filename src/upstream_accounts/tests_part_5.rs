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
    let excluded_blocked = insert_test_pool_api_key_account_with_options(
        &state,
        "Excluded Route Blocked",
        "sk-excluded-route-blocked",
        None,
        Some("https://same-route.example.com/backend-api/codex"),
    )
    .await;
    let alternate_blocked = insert_test_pool_api_key_account_with_options(
        &state,
        "Alternate Route Blocked",
        "sk-alternate-route-blocked",
        None,
        Some("https://alternate-route.example.com/backend-api/codex"),
    )
    .await;
    set_test_account_group_name(&state.pool, excluded_blocked, Some("same-route-missing")).await;
    set_test_account_group_name(&state.pool, alternate_blocked, Some("alternate-missing")).await;
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
    assert_eq!(
        message,
        "upstream account group \"alternate-missing\" has no bound forward proxy nodes; bind at least one proxy node to the group"
    );
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
    let no_cut_in_tag = insert_tag(
        &state.pool,
        "excluded-route-no-cut-in",
        &TagRoutingRule {
            guard_enabled: false,
            lookback_hours: None,
            max_conversations: None,
            allow_cut_out: true,
            allow_cut_in: false,
            priority_tier: TagPriorityTier::Normal,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            concurrency_limit: 0,
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
    let PoolAccountResolution::BlockedByPolicy(message) = resolution else {
        panic!("expected alternate group proxy error to survive excluded cut-in reject");
    };
    assert_eq!(
        message,
        "upstream account group \"alternate-missing\" has no bound forward proxy nodes; bind at least one proxy node to the group"
    );
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
    let lock_tag = insert_tag(
        &state.pool,
        "sticky-lock",
        &TagRoutingRule {
            guard_enabled: false,
            lookback_hours: None,
            max_conversations: None,
            allow_cut_out: false,
            allow_cut_in: true,
            priority_tier: TagPriorityTier::Normal,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            concurrency_limit: 0,
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
    let lock_tag = insert_tag(
        &state.pool,
        "sticky-ungrouped-lock",
        &TagRoutingRule {
            guard_enabled: false,
            lookback_hours: None,
            max_conversations: None,
            allow_cut_out: false,
            allow_cut_in: true,
            priority_tier: TagPriorityTier::Normal,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            concurrency_limit: 0,
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
    let lock_tag = insert_tag(
        &state.pool,
        "sticky-cut-out-forbidden",
        &TagRoutingRule {
            guard_enabled: false,
            lookback_hours: None,
            max_conversations: None,
            allow_cut_out: false,
            allow_cut_in: true,
            priority_tier: TagPriorityTier::Normal,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            concurrency_limit: 0,
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
        "sticky conversation cannot cut out of the current account because a tag rule forbids it"
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
    let no_cut_in_tag = insert_tag(
        &state.pool,
        "sticky-no-cut-in",
        &TagRoutingRule {
            guard_enabled: false,
            lookback_hours: None,
            max_conversations: None,
            allow_cut_out: true,
            allow_cut_in: false,
            priority_tier: TagPriorityTier::Normal,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            concurrency_limit: 0,
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
async fn update_oauth_login_session_preserves_pending_url_and_persists_metadata() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let tag_id = insert_tag(&state.pool, "pending-sync", &test_tag_routing_rule())
        .await
        .expect("insert tag")
        .summary
        .id;
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
            group_name: Some("alpha".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
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
            group_name: OptionalField::Value("beta".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            note: OptionalField::Value("after".to_string()),
            group_note: OptionalField::Value("beta shared".to_string()),
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![tag_id]),
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
        vec![tag_id]
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
async fn update_oauth_login_session_ignores_stale_baseline_updates() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Ordered Pending".to_string()),
            group_name: Some("alpha".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
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
            group_name: OptionalField::Value("beta".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
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
            group_name: OptionalField::Value("gamma".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
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
async fn update_oauth_login_session_preserves_omitted_fields() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let tag_id = insert_tag(&state.pool, "partial-sync", &test_tag_routing_rule())
        .await
        .expect("insert tag")
        .summary
        .id;
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
            group_name: Some("partial-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
            note: Some("before partial patch".to_string()),
            group_note: Some("partial draft note".to_string()),
            concurrency_limit: None,
            account_id: None,
            tag_ids: vec![tag_id],
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
            group_name: OptionalField::Missing,
            group_bound_proxy_keys: OptionalField::Missing,
            group_node_shunt_enabled: OptionalField::Missing,
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
        vec![tag_id]
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
async fn update_oauth_login_session_clears_omitted_group_note_when_group_changes() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Move Draft Group".to_string()),
            group_name: Some("before-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
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
            group_name: OptionalField::Value("after-group".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
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
async fn update_oauth_login_session_rejects_group_removal() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Clear Group Note".to_string()),
            group_name: Some("draft-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
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

    let updated = update_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Missing,
            group_name: OptionalField::Value(String::new()),
            group_bound_proxy_keys: OptionalField::Value(vec![]),
            group_node_shunt_enabled: OptionalField::Missing,
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
    .expect_err("group removal should be rejected");
    assert_eq!(updated.0, StatusCode::BAD_REQUEST);
    assert_eq!(updated.1, "groupName is required for upstream accounts");

    let stored = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("load stored login session")
        .expect("stored login session should exist");
    assert_eq!(stored.group_name.as_deref(), Some("draft-group"));
    assert_eq!(stored.group_note.as_deref(), Some("draft group note"));
    assert_eq!(stored.note.as_deref(), Some("before clearing group"));
}

#[tokio::test]
async fn updated_oauth_login_session_metadata_is_used_when_callback_persists_account() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let tag_id = insert_tag(&state.pool, "callback-sync", &test_tag_routing_rule())
        .await
        .expect("insert tag")
        .summary
        .id;
    insert_test_oauth_mailbox_session(
        &state.pool,
        "mailbox-session-2",
        "callback-sync@mail-tw.707079.xyz",
        OAUTH_MAILBOX_SOURCE_ATTACHED,
    )
    .await;

    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Before Patch".to_string()),
            group_name: Some("old-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
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
    .0;

    let _ = update_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("After Patch".to_string()),
            group_name: OptionalField::Value("new-group".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            note: OptionalField::Value("after note".to_string()),
            group_note: OptionalField::Value("draft group note".to_string()),
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![tag_id]),
            is_mother: OptionalField::Value(true),
            mailbox_session_id: OptionalField::Value("mailbox-session-2".to_string()),
            mailbox_address: OptionalField::Value("callback-sync@mail-tw.707079.xyz".to_string()),
        }),
    )
    .await
    .expect("update oauth login session");

    let updated_session = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("load updated session")
        .expect("updated session should exist");
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let encrypted_credentials = encrypt_credentials(
        crypto_key,
        &StoredCredentials::Oauth(StoredOauthCredentials {
            access_token: "callback-access".to_string(),
            refresh_token: "callback-refresh".to_string(),
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
    let account_id = persist_oauth_callback_inner(
        state.as_ref(),
        PersistOauthCallbackInput {
            display_name: updated_session
                .display_name
                .clone()
                .expect("display name should be stored"),
            session: updated_session.clone(),
            claims: test_claims(
                "callback@example.com",
                Some("org_callback"),
                Some("user_callback"),
            ),
            encrypted_credentials,
            token_expires_at: "2026-04-01T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("persist oauth callback");

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
    assert_eq!(account_tag_ids, vec![tag_id]);

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

    let completed_session = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("load completed session")
        .expect("completed session should exist");
    assert_eq!(completed_session.status, LOGIN_SESSION_STATUS_COMPLETED);
    assert_eq!(completed_session.account_id, Some(account_id));
}

#[tokio::test]
async fn persist_oauth_callback_preserves_group_node_shunt_for_legacy_pending_session() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
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
            group_name: Some("legacy-group".to_string()),
            group_bound_proxy_keys: Some(bound_proxy_keys.clone()),
            group_node_shunt_enabled: None,
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
        r#"
            UPDATE pool_oauth_login_sessions
            SET group_node_shunt_enabled = 0,
                group_node_shunt_enabled_requested = 0
            WHERE login_id = ?1
            "#,
    )
    .bind(&created.login_id)
    .execute(&state.pool)
    .await
    .expect("downgrade pending session to legacy node shunt fields");

    let pending_session = load_login_session_by_login_id(&state.pool, &created.login_id)
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
            refresh_token: "legacy-refresh".to_string(),
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
    let account_id = persist_oauth_callback_inner(
        state.as_ref(),
        PersistOauthCallbackInput {
            display_name: pending_session
                .display_name
                .clone()
                .expect("display name should be stored"),
            session: pending_session,
            claims: test_claims(
                "legacy@example.com",
                Some("org_legacy"),
                Some("user_legacy"),
            ),
            encrypted_credentials,
            token_expires_at: "2026-04-01T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("persist oauth callback");

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

#[tokio::test]
async fn update_oauth_login_session_repairs_completed_callback_race_with_latest_metadata() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let tag_id = insert_tag(&state.pool, "callback-race-sync", &test_tag_routing_rule())
        .await
        .expect("insert tag")
        .summary
        .id;

    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Race Before".to_string()),
            group_name: Some("race-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
            note: Some("before note".to_string()),
            group_note: Some("before group note".to_string()),
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

    let pending_session = load_login_session_by_login_id(&state.pool, &created.login_id)
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
            access_token: "race-access".to_string(),
            refresh_token: "race-refresh".to_string(),
            id_token: test_id_token(
                "race@example.com",
                Some("org_race"),
                Some("user_race"),
                Some("team"),
            ),
            token_type: Some("Bearer".to_string()),
        }),
    )
    .expect("encrypt oauth credentials");
    let account_id = persist_oauth_callback_inner(
        state.as_ref(),
        PersistOauthCallbackInput {
            display_name: pending_session
                .display_name
                .clone()
                .expect("display name should be stored"),
            session: pending_session,
            claims: test_claims("race@example.com", Some("org_race"), Some("user_race")),
            encrypted_credentials,
            token_expires_at: "2026-04-01T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("persist oauth callback");

    let mut repair_headers = HeaderMap::new();
    repair_headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(&created.updated_at).expect("valid updated_at header"),
    );
    let repaired = update_oauth_login_session(
        State(state.clone()),
        repair_headers,
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("Race After".to_string()),
            group_name: OptionalField::Value("race-group".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            note: OptionalField::Value("after note".to_string()),
            group_note: OptionalField::Value("after group note".to_string()),
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![tag_id]),
            is_mother: OptionalField::Value(true),
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect("repair completed callback race")
    .0;

    assert_eq!(repaired.login_id, created.login_id);
    assert_eq!(repaired.status, LOGIN_SESSION_STATUS_COMPLETED);
    assert_eq!(repaired.account_id, Some(account_id));
    assert!(repaired.auth_url.is_none());
    assert!(repaired.redirect_uri.is_none());

    let account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load repaired account row")
        .expect("oauth account should exist");
    assert_eq!(account.display_name, "Race After");
    assert_eq!(account.group_name.as_deref(), Some("race-group"));
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
    .expect("load repaired oauth account tags");
    assert_eq!(account_tag_ids, vec![tag_id]);

    let group_note = sqlx::query_scalar::<_, Option<String>>(
        r#"
            SELECT note
            FROM pool_upstream_account_group_notes
            WHERE group_name = ?1
            "#,
    )
    .bind("race-group")
    .fetch_one(&state.pool)
    .await
    .expect("load repaired group note");
    assert_eq!(group_note.as_deref(), Some("after group note"));

    let completed_session = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("reload completed session")
        .expect("completed session should still exist");
    assert_ne!(completed_session.updated_at, created.updated_at);
    assert!(completed_session.consumed_at.is_some());

    let mut second_repair_headers = HeaderMap::new();
    second_repair_headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(&repaired.updated_at).expect("valid updated_at header"),
    );
    let second_repair = update_oauth_login_session(
        State(state.clone()),
        second_repair_headers,
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("Race Final".to_string()),
            group_name: OptionalField::Missing,
            group_bound_proxy_keys: OptionalField::Missing,
            group_node_shunt_enabled: OptionalField::Missing,
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
    .expect("repair completed callback race again with omitted fields")
    .0;

    let repaired_again = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load twice repaired account row")
        .expect("oauth account should still exist");
    assert_eq!(repaired_again.display_name, "Race Final");
    assert_eq!(repaired_again.group_name.as_deref(), Some("race-group"));
    assert_eq!(repaired_again.note.as_deref(), Some("after note"));
    assert_eq!(repaired_again.is_mother, 1);

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
    .expect("load twice repaired oauth account tags");
    assert_eq!(account_tag_ids, vec![tag_id]);

    let second_group_note = sqlx::query_scalar::<_, Option<String>>(
        r#"
            SELECT note
            FROM pool_upstream_account_group_notes
            WHERE group_name = ?1
            "#,
    )
    .bind("race-group")
    .fetch_one(&state.pool)
    .await
    .expect("load twice repaired group note");
    assert_eq!(second_group_note.as_deref(), Some("after group note"));

    let repaired_session = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("reload repaired session after second patch")
        .expect("repaired session should still exist");
    assert_eq!(repaired_session.display_name.as_deref(), Some("Race Final"));
    assert_eq!(repaired_session.group_name.as_deref(), Some("race-group"));
    assert_eq!(repaired_session.note.as_deref(), Some("after note"));
    assert_eq!(
        parse_tag_ids_json(repaired_session.tag_ids_json.as_deref()),
        vec![tag_id]
    );
    assert_ne!(second_repair.updated_at, repaired.updated_at);
}

#[tokio::test]
async fn update_oauth_login_session_rejects_stale_completed_race_repairs() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Race Before".to_string()),
            group_name: Some("race-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
            note: Some("before note".to_string()),
            group_note: Some("before group note".to_string()),
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

    let pending_session = load_login_session_by_login_id(&state.pool, &created.login_id)
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
            access_token: "race-access".to_string(),
            refresh_token: "race-refresh".to_string(),
            id_token: test_id_token(
                "race@example.com",
                Some("org_race"),
                Some("user_race"),
                Some("team"),
            ),
            token_type: Some("Bearer".to_string()),
        }),
    )
    .expect("encrypt oauth credentials");
    let account_id = persist_oauth_callback_inner(
        state.as_ref(),
        PersistOauthCallbackInput {
            display_name: pending_session
                .display_name
                .clone()
                .expect("display name should be stored"),
            session: pending_session,
            claims: test_claims("race@example.com", Some("org_race"), Some("user_race")),
            encrypted_credentials,
            token_expires_at: "2026-04-01T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("persist oauth callback");

    let completed_session = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("load completed session")
        .expect("completed session should exist");
    assert_eq!(completed_session.updated_at, created.updated_at);
    assert!(completed_session.consumed_at.is_some());

    let mut first_headers = HeaderMap::new();
    first_headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(&created.updated_at).expect("valid updated_at header"),
    );
    let first_repair = update_oauth_login_session(
        State(state.clone()),
        first_headers,
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("Race Latest".to_string()),
            group_name: OptionalField::Value("race-group".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            note: OptionalField::Value("latest note".to_string()),
            group_note: OptionalField::Value("latest group note".to_string()),
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![]),
            is_mother: OptionalField::Value(true),
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect("apply latest repair")
    .0;

    assert_ne!(first_repair.updated_at, created.updated_at);
    assert_eq!(first_repair.account_id, Some(account_id));

    let mut stale_headers = HeaderMap::new();
    stale_headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(&created.updated_at).expect("valid updated_at header"),
    );
    let stale_err = update_oauth_login_session(
        State(state.clone()),
        stale_headers,
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("Race Stale".to_string()),
            group_name: OptionalField::Value("stale-group".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            note: OptionalField::Value("stale note".to_string()),
            group_note: OptionalField::Value("stale group note".to_string()),
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![]),
            is_mother: OptionalField::Value(false),
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect_err("reject stale repair");
    assert_eq!(stale_err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        stale_err.1,
        "This login session can no longer be edited.".to_string()
    );

    let account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load account after stale repair rejection")
        .expect("oauth account should exist");
    assert_eq!(account.display_name, "Race Latest");
    assert_eq!(account.group_name.as_deref(), Some("race-group"));
    assert_eq!(account.note.as_deref(), Some("latest note"));
    assert_eq!(account.is_mother, 1);
}

#[tokio::test]
async fn update_oauth_login_session_rejects_completed_repairs_after_group_note_changes() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Race Before".to_string()),
            group_name: Some("race-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
            note: Some("before note".to_string()),
            group_note: Some("before group note".to_string()),
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

    let pending_session = load_login_session_by_login_id(&state.pool, &created.login_id)
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
            access_token: "race-access".to_string(),
            refresh_token: "race-refresh".to_string(),
            id_token: test_id_token(
                "race@example.com",
                Some("org_race"),
                Some("user_race"),
                Some("team"),
            ),
            token_type: Some("Bearer".to_string()),
        }),
    )
    .expect("encrypt oauth credentials");
    let account_id = persist_oauth_callback_inner(
        state.as_ref(),
        PersistOauthCallbackInput {
            display_name: pending_session
                .display_name
                .clone()
                .expect("display name should be stored"),
            session: pending_session,
            claims: test_claims("race@example.com", Some("org_race"), Some("user_race")),
            encrypted_credentials,
            token_expires_at: "2026-04-01T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("persist oauth callback");

    let completed_session = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("load completed session")
        .expect("completed session should exist");
    assert_eq!(completed_session.updated_at, created.updated_at);

    let mut conn = state.pool.acquire().await.expect("acquire group note conn");
    save_group_note_record_conn(
        &mut conn,
        "race-group",
        Some("manual latest group note".to_string()),
    )
    .await
    .expect("save manual latest group note");
    drop(conn);

    let mut repair_headers = HeaderMap::new();
    repair_headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(&created.updated_at).expect("valid updated_at header"),
    );
    let repair_err = update_oauth_login_session(
        State(state.clone()),
        repair_headers,
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("Race Latest".to_string()),
            group_name: OptionalField::Value("race-group".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            note: OptionalField::Value("latest note".to_string()),
            group_note: OptionalField::Value("latest group note".to_string()),
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![]),
            is_mother: OptionalField::Value(true),
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect_err("reject repair after group note changes");
    assert_eq!(repair_err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        repair_err.1,
        "This login session can no longer be edited.".to_string()
    );

    let account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load account after repair rejection")
        .expect("oauth account should exist");
    assert_eq!(account.display_name, "Race Before");
    assert_eq!(account.group_name.as_deref(), Some("race-group"));
    assert_eq!(account.note.as_deref(), Some("before note"));

    let group_note = sqlx::query_scalar::<_, Option<String>>(
        r#"
            SELECT note
            FROM pool_upstream_account_group_notes
            WHERE group_name = ?1
            "#,
    )
    .bind("race-group")
    .fetch_one(&state.pool)
    .await
    .expect("load preserved group note");
    assert_eq!(group_note.as_deref(), Some("manual latest group note"));
}

#[tokio::test]
async fn update_oauth_login_session_rejects_completed_repairs_after_account_changes() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Race Before".to_string()),
            group_name: Some("race-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
            note: Some("before note".to_string()),
            group_note: Some("before group note".to_string()),
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

    let pending_session = load_login_session_by_login_id(&state.pool, &created.login_id)
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
            access_token: "race-access".to_string(),
            refresh_token: "race-refresh".to_string(),
            id_token: test_id_token(
                "race@example.com",
                Some("org_race"),
                Some("user_race"),
                Some("team"),
            ),
            token_type: Some("Bearer".to_string()),
        }),
    )
    .expect("encrypt oauth credentials");
    let account_id = persist_oauth_callback_inner(
        state.as_ref(),
        PersistOauthCallbackInput {
            display_name: pending_session
                .display_name
                .clone()
                .expect("display name should be stored"),
            session: pending_session,
            claims: test_claims("race@example.com", Some("org_race"), Some("user_race")),
            encrypted_credentials,
            token_expires_at: "2026-04-01T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("persist oauth callback");

    let completed_session = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("load completed session")
        .expect("completed session should exist");
    let consumed_at = completed_session
        .consumed_at
        .clone()
        .expect("completed session should record consumed_at");
    let newer_account_updated_at = next_login_session_updated_at(Some(consumed_at.as_str()));
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET display_name = ?2,
                note = ?3,
                updated_at = ?4
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind("Manual Latest")
    .bind("manual latest note")
    .bind(&newer_account_updated_at)
    .execute(&state.pool)
    .await
    .expect("simulate newer account edit");

    let mut repair_headers = HeaderMap::new();
    repair_headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(&created.updated_at).expect("valid updated_at header"),
    );
    let repair_err = update_oauth_login_session(
        State(state.clone()),
        repair_headers,
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("Race Stale".to_string()),
            group_name: OptionalField::Value("stale-group".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            note: OptionalField::Value("stale note".to_string()),
            group_note: OptionalField::Value("stale group note".to_string()),
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![]),
            is_mother: OptionalField::Value(false),
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect_err("reject completed repair after account changes");
    assert_eq!(repair_err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        repair_err.1,
        "This login session can no longer be edited.".to_string()
    );

    let account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load account after rejecting stale completed repair")
        .expect("oauth account should exist");
    assert_eq!(account.display_name, "Manual Latest");
    assert_eq!(account.group_name.as_deref(), Some("race-group"));
    assert_eq!(account.note.as_deref(), Some("manual latest note"));
    assert_eq!(account.updated_at, newer_account_updated_at);
}

#[tokio::test]
async fn update_oauth_login_session_rejects_completed_failed_and_expired_sessions() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let update_payload = || UpdateOauthLoginSessionRequest {
        display_name: OptionalField::Value("Edited Session".to_string()),
        group_name: OptionalField::Value("edited-group".to_string()),
        group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
        group_node_shunt_enabled: OptionalField::Missing,
        note: OptionalField::Value("edited note".to_string()),
        group_note: OptionalField::Value("edited group note".to_string()),
        concurrency_limit: OptionalField::Missing,
        tag_ids: OptionalField::Value(vec![]),
        is_mother: OptionalField::Value(false),
        mailbox_session_id: OptionalField::Missing,
        mailbox_address: OptionalField::Missing,
    };

    let completed = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Completed Session".to_string()),
            group_name: Some("completed-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
            note: None,
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
    .expect("create completed session seed")
    .0;
    sqlx::query("UPDATE pool_oauth_login_sessions SET status = ?2 WHERE login_id = ?1")
        .bind(&completed.login_id)
        .bind(LOGIN_SESSION_STATUS_COMPLETED)
        .execute(&state.pool)
        .await
        .expect("mark session completed");
    let completed_err = update_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(completed.login_id.clone()),
        Json(update_payload()),
    )
    .await
    .expect_err("completed session should reject edits");
    assert_eq!(completed_err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        completed_err.1,
        "This login session can no longer be edited."
    );

    let failed = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Failed Session".to_string()),
            group_name: Some("failed-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
            note: None,
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
    .expect("create failed session seed")
    .0;
    sqlx::query("UPDATE pool_oauth_login_sessions SET status = ?2 WHERE login_id = ?1")
        .bind(&failed.login_id)
        .bind(LOGIN_SESSION_STATUS_FAILED)
        .execute(&state.pool)
        .await
        .expect("mark session failed");
    let failed_err = update_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(failed.login_id.clone()),
        Json(update_payload()),
    )
    .await
    .expect_err("failed session should reject edits");
    assert_eq!(failed_err.0, StatusCode::BAD_REQUEST);
    assert_eq!(failed_err.1, "This login session can no longer be edited.");

    let expired = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Expired Session".to_string()),
            group_name: Some("expired-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
            note: None,
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
    .expect("create expired session seed")
    .0;
    sqlx::query("UPDATE pool_oauth_login_sessions SET expires_at = ?2 WHERE login_id = ?1")
        .bind(&expired.login_id)
        .bind("2020-01-01T00:00:00Z")
        .execute(&state.pool)
        .await
        .expect("expire session");
    let expired_err = update_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(expired.login_id.clone()),
        Json(update_payload()),
    )
    .await
    .expect_err("expired session should reject edits");
    assert_eq!(expired_err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        expired_err.1,
        "The login session has expired. Please create a new authorization link."
    );

    let expired_session = load_login_session_by_login_id(&state.pool, &expired.login_id)
        .await
        .expect("load expired session")
        .expect("expired session should exist");
    assert_eq!(expired_session.status, LOGIN_SESSION_STATUS_EXPIRED);
}

#[tokio::test]
async fn update_oauth_login_session_rejects_relogin_sessions() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_oauth_account(&state.pool, "Relogin Target").await;
    let relogin = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: None,
            group_name: None,
            group_bound_proxy_keys: None,
            group_node_shunt_enabled: None,
            note: None,
            group_note: None,
            concurrency_limit: None,
            account_id: Some(account_id),
            tag_ids: vec![],
            is_mother: Some(false),
            mailbox_session_id: None,
            mailbox_address: None,
        }),
    )
    .await
    .expect("create relogin session")
    .0;

    let err = update_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(relogin.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("Edited Relogin".to_string()),
            group_name: OptionalField::Missing,
            group_bound_proxy_keys: OptionalField::Missing,
            group_node_shunt_enabled: OptionalField::Missing,
            note: OptionalField::Missing,
            group_note: OptionalField::Missing,
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![]),
            is_mother: OptionalField::Value(false),
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect_err("relogin session should reject edits");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        err.1,
        "This login session belongs to an existing account and cannot be edited."
    );
}
