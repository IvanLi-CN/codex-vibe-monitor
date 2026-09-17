use super::*;
use crate::tests::insert_test_pool_oauth_account;

#[tokio::test]
pub(crate) async fn resolver_skips_no_new_priority_for_fresh_routing_without_sticky_key() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let blocked = insert_test_pool_api_key_account_with_options(
        &state,
        "Blocked Fresh Route",
        "sk-blocked-fresh-route",
        None,
        Some("https://blocked-fresh.example.com/backend-api/codex"),
    )
    .await;
    let fallback = insert_test_pool_api_key_account_with_options(
        &state,
        "Allowed Fresh Route",
        "sk-allowed-fresh-route",
        None,
        Some("https://allowed-fresh.example.com/backend-api/codex"),
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET policy_priority_tier = 'no_new' WHERE id = ?1")
        .bind(blocked)
        .execute(&state.pool)
        .await
        .expect("block fresh route");
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, blocked, &now_iso, Some(1.0), Some(1.0)).await;
    insert_limit_sample_with_usage(&state.pool, fallback, &now_iso, Some(80.0), Some(20.0)).await;

    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected allowed fallback account");
    };
    assert_eq!(account.account_id, fallback);
}

#[tokio::test]
pub(crate) async fn resolver_allows_existing_sticky_reuse_for_no_new_priority_account() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let blocked = insert_test_pool_api_key_account_with_options(
        &state,
        "Blocked Sticky Reuse",
        "sk-blocked-sticky-reuse",
        None,
        Some("https://blocked-sticky.example.com/backend-api/codex"),
    )
    .await;
    let fallback = insert_test_pool_api_key_account_with_options(
        &state,
        "Ignored Sticky Fallback",
        "sk-ignored-sticky-fallback",
        None,
        Some("https://ignored-sticky.example.com/backend-api/codex"),
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET policy_priority_tier = 'no_new' WHERE id = ?1")
        .bind(blocked)
        .execute(&state.pool)
        .await
        .expect("block fresh route");
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, blocked, &now_iso, Some(1.0), Some(1.0)).await;
    insert_limit_sample_with_usage(&state.pool, fallback, &now_iso, Some(80.0), Some(20.0)).await;
    upsert_sticky_route(&state.pool, "blocked-sticky-reuse", blocked, &now_iso)
        .await
        .expect("upsert sticky route");

    let resolution = resolve_pool_account_for_request(
        &state,
        Some("blocked-sticky-reuse"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected existing sticky account");
    };
    assert_eq!(account.account_id, blocked);
}

#[tokio::test]
pub(crate) async fn resolver_demotes_recent_timeout_for_same_upstream_route_and_proxy_binding() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let primary_route = "https://primary-timeout.example.com/backend-api/codex";
    let alternate_route = "https://alternate-timeout.example.com/backend-api/codex";
    let primary = insert_test_pool_api_key_account_with_options(
        &state,
        "Primary Timeout Combo",
        "sk-primary-timeout-combo",
        None,
        Some(primary_route),
    )
    .await;
    let alternate = insert_test_pool_api_key_account_with_options(
        &state,
        "Alternate Timeout Combo",
        "sk-alternate-timeout-combo",
        None,
        Some(alternate_route),
    )
    .await;
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, primary, &now_iso, Some(1.0), Some(1.0)).await;
    insert_limit_sample_with_usage(&state.pool, alternate, &now_iso, Some(80.0), Some(20.0)).await;
    seed_route_binding_attempt(
        &state.pool,
        "route-binding-timeout",
        primary_route,
        FORWARD_PROXY_DIRECT_KEY,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
        Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT),
    )
    .await;

    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected resolved alternate account");
    };
    assert_eq!(account.account_id, alternate);
}

#[tokio::test]
pub(crate) async fn resolver_does_not_demote_successful_or_non_timeout_route_proxy_history() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let primary_route = "https://primary-success.example.com/backend-api/codex";
    let alternate_route = "https://alternate-success.example.com/backend-api/codex";
    let primary = insert_test_pool_api_key_account_with_options(
        &state,
        "Primary Successful Combo",
        "sk-primary-success-combo",
        None,
        Some(primary_route),
    )
    .await;
    let alternate = insert_test_pool_api_key_account_with_options(
        &state,
        "Alternate Successful Combo",
        "sk-alternate-success-combo",
        None,
        Some(alternate_route),
    )
    .await;
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, primary, &now_iso, Some(1.0), Some(1.0)).await;
    insert_limit_sample_with_usage(&state.pool, alternate, &now_iso, Some(80.0), Some(20.0)).await;
    seed_route_binding_attempt(
        &state.pool,
        "route-binding-auth-failure",
        primary_route,
        FORWARD_PROXY_DIRECT_KEY,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
        Some(PROXY_FAILURE_UPSTREAM_HTTP_AUTH),
    )
    .await;
    seed_route_binding_attempt(
        &state.pool,
        "route-binding-success",
        primary_route,
        FORWARD_PROXY_DIRECT_KEY,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        None,
    )
    .await;

    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected resolved primary account");
    };
    assert_eq!(account.account_id, primary);
}

#[tokio::test]
pub(crate) async fn resolver_reuses_sticky_account_when_cut_out_is_forbidden_despite_recent_route_binding_penalty()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let sticky_route = "https://sticky-penalized.example.com/backend-api/codex";
    let fallback_route = "https://sticky-fallback.example.com/backend-api/codex";
    let sticky_account = insert_test_pool_api_key_account_with_options(
        &state,
        "Sticky Penalized Owner",
        "sk-sticky-penalized-owner",
        None,
        Some(sticky_route),
    )
    .await;
    let fallback_account = insert_test_pool_api_key_account_with_options(
        &state,
        "Sticky Penalized Fallback",
        "sk-sticky-penalized-fallback",
        None,
        Some(fallback_route),
    )
    .await;
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(
        &state.pool,
        sticky_account,
        &now_iso,
        Some(30.0),
        Some(30.0),
    )
    .await;
    insert_limit_sample_with_usage(
        &state.pool,
        fallback_account,
        &now_iso,
        Some(1.0),
        Some(1.0),
    )
    .await;
    let lock_tag = insert_test_tag(
        &state.pool,
        "sticky-penalty-lock",
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
        "sticky-penalty-cut-out-forbidden",
        sticky_account,
        &now_iso,
    )
    .await
    .expect("upsert sticky route");
    seed_route_binding_attempt(
        &state.pool,
        "sticky-penalty-cut-out-forbidden-attempt",
        sticky_route,
        FORWARD_PROXY_DIRECT_KEY,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
        Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT),
    )
    .await;

    let resolution = resolve_pool_account_for_request(
        &state,
        Some("sticky-penalty-cut-out-forbidden"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected sticky owner to stay live when cut out is forbidden");
    };
    assert_eq!(account.account_id, sticky_account);
}

#[tokio::test]
pub(crate) async fn resolver_preserves_sticky_hard_block_when_cut_out_is_forbidden_despite_recent_route_binding_penalty()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let sticky_route = "https://sticky-hard-block.example.com/backend-api/codex";
    let sticky_account = insert_test_pool_oauth_account(
        &state,
        "Sticky Hard Block Owner",
        "oauth-sticky-hard-block-owner",
    )
    .await;
    let _fallback_account = insert_test_pool_oauth_account(
        &state,
        "Sticky Hard Block Fallback",
        "oauth-sticky-hard-block-fallback",
    )
    .await;
    for (account_id, upstream_base_url) in [
        (sticky_account, sticky_route),
        (
            _fallback_account,
            "https://sticky-hard-block-fallback.example.com/backend-api/codex",
        ),
    ] {
        sqlx::query("UPDATE pool_upstream_accounts SET upstream_base_url = ?2 WHERE id = ?1")
            .bind(account_id)
            .bind(upstream_base_url)
            .execute(&state.pool)
            .await
            .expect("set sticky route upstream base");
    }
    set_test_account_group_name(&state.pool, sticky_account, Some("sticky-penalty-missing")).await;
    let now_iso = format_utc_iso(Utc::now());
    let lock_tag = insert_test_tag(
        &state.pool,
        "sticky-hard-block-lock",
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
        "sticky-hard-block-cut-out-forbidden",
        sticky_account,
        &now_iso,
    )
    .await
    .expect("upsert sticky route");
    seed_route_binding_attempt(
        &state.pool,
        "sticky-hard-block-cut-out-forbidden-attempt",
        sticky_route,
        FORWARD_PROXY_DIRECT_KEY,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
        Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT),
    )
    .await;

    let resolution = resolve_pool_account_for_request(
        &state,
        Some("sticky-hard-block-cut-out-forbidden"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve pool account");
    let PoolAccountResolution::AssignedBlocked(blocked) = resolution else {
        panic!("expected sticky hard block to preserve the assigned account");
    };
    assert_eq!(blocked.account.account_id, sticky_account);
    assert_eq!(
        blocked.failure_kind,
        PROXY_FAILURE_POOL_ASSIGNED_ACCOUNT_BLOCKED
    );
    assert_eq!(
        blocked.message,
        "upstream account group \"sticky-penalty-missing\" has no bound forward proxy nodes; bind at least one proxy node to the group"
    );
}

#[tokio::test]
pub(crate) async fn resolver_applies_prompt_cache_group_binding_as_hard_constraint() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let preferred_group = "prompt-cache-bound-group";
    let other_group = "prompt-cache-other-group";
    let preferred = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Bound Group",
        "sk-prompt-cache-bound-group",
        Some(preferred_group),
        Some("https://bound-group.example.com/backend-api/codex"),
    )
    .await;
    let other = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Other Group",
        "sk-prompt-cache-other-group",
        Some(other_group),
        Some("https://other-group.example.com/backend-api/codex"),
    )
    .await;
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, preferred, &now_iso, Some(20.0), Some(20.0)).await;
    insert_limit_sample_with_usage(&state.pool, other, &now_iso, Some(1.0), Some(1.0)).await;

    let resolution = resolve_pool_account_for_request_with_binding_constraint(
        &state,
        None,
        &[],
        &HashSet::new(),
        Some(&PromptCacheConversationBindingConstraint::Group(
            preferred_group.to_string(),
        )),
    )
    .await
    .expect("resolve group-bound pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected group-bound account");
    };
    assert_eq!(account.account_id, preferred);
    assert_eq!(account.group_name.as_deref(), Some(preferred_group));
}

#[tokio::test]
pub(crate) async fn resolver_non_explicit_sticky_escape_cuts_out_after_two_recent_upstream_stream_errors()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let unhealthy_route = "https://non-explicit-escape-unhealthy.example.com/backend-api/codex";
    let healthy_route = "https://non-explicit-escape-healthy.example.com/backend-api/codex";
    let unhealthy = insert_test_pool_api_key_account_with_options(
        &state,
        "Non Explicit Escape Unhealthy",
        "sk-non-explicit-escape-unhealthy",
        Some(test_required_group_name()),
        Some(unhealthy_route),
    )
    .await;
    let healthy = insert_test_pool_api_key_account_with_options(
        &state,
        "Non Explicit Escape Healthy",
        "sk-non-explicit-escape-healthy",
        Some(test_required_group_name()),
        Some(healthy_route),
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET policy_allow_cut_out = 0 WHERE id = ?1")
        .bind(unhealthy)
        .execute(&state.pool)
        .await
        .expect("lock unhealthy sticky source");
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, unhealthy, &now_iso, Some(1.0), Some(1.0)).await;
    insert_limit_sample_with_usage(&state.pool, healthy, &now_iso, Some(40.0), Some(20.0)).await;
    upsert_sticky_route(&state.pool, "non-explicit-escaped-key", unhealthy, &now_iso)
        .await
        .expect("seed non-explicit sticky route");
    seed_account_upstream_stream_error_attempt(
        &state.pool,
        "non-explicit-stream-error-1",
        "non-explicit-old-key-a",
        test_required_group_name(),
        unhealthy,
        unhealthy_route,
    )
    .await;
    seed_account_upstream_stream_error_attempt(
        &state.pool,
        "non-explicit-stream-error-2",
        "non-explicit-old-key-b",
        test_required_group_name(),
        unhealthy,
        unhealthy_route,
    )
    .await;

    let sticky_resolution = resolve_pool_account_for_request(
        &state,
        Some("non-explicit-escaped-key"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve escaped sticky account");
    let PoolAccountResolution::Resolved(sticky_account) = sticky_resolution else {
        panic!("expected non-explicit sticky route to escape to a healthy account");
    };
    assert_eq!(sticky_account.account_id, healthy);
    assert_eq!(
        sticky_account.routing_source,
        PoolRoutingSelectionSource::FreshAssignment
    );

    let already_tried_resolution = resolve_pool_account_for_request(
        &state,
        Some("non-explicit-escaped-key"),
        &[unhealthy],
        &HashSet::new(),
    )
    .await
    .expect("resolve escaped sticky account after the source was tried");
    let PoolAccountResolution::Resolved(already_tried_account) = already_tried_resolution else {
        panic!("expected an active escape to bypass the sticky cut-out guard");
    };
    assert_eq!(already_tried_account.account_id, healthy);

    let fresh_resolution = resolve_pool_account_for_request(
        &state,
        Some("non-explicit-fresh-key"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve escaped fresh account");
    let PoolAccountResolution::Resolved(fresh_account) = fresh_resolution else {
        panic!("expected new non-explicit sticky target to avoid the unhealthy account");
    };
    assert_eq!(fresh_account.account_id, healthy);
}

#[tokio::test]
pub(crate) async fn transport_decode_sticky_escape_state_expires_at_latest_failure_plus_window() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let route = "https://bounded-stream-escape.example.com/backend-api/codex";
    let account = insert_test_pool_api_key_account_with_options(
        &state,
        "Bounded Stream Escape",
        "sk-bounded-stream-escape",
        Some(test_required_group_name()),
        Some(route),
    )
    .await;
    let now = parse_rfc3339_utc("2026-07-26T12:00:00Z").expect("fixed now");
    seed_account_attempt_at(
        &state.pool,
        AccountAttemptSeed {
            invoke_id: "bounded-stream-escape-1",
            sticky_key: "bounded-stream-escape-key-1",
            group_name: test_required_group_name(),
            upstream_account_id: account,
            upstream_base_url: route,
            occurred_at: &format_naive(
                (now - ChronoDuration::seconds(299))
                    .with_timezone(&Shanghai)
                    .naive_local(),
            ),
            status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
            phase: POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED,
            failure_kind: Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR),
        },
    )
    .await;
    seed_account_attempt_at(
        &state.pool,
        AccountAttemptSeed {
            invoke_id: "bounded-stream-escape-2",
            sticky_key: "bounded-stream-escape-key-2",
            group_name: test_required_group_name(),
            upstream_account_id: account,
            upstream_base_url: route,
            occurred_at: &format_naive(
                (now - ChronoDuration::seconds(1))
                    .with_timezone(&Shanghai)
                    .naive_local(),
            ),
            status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
            phase: POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED,
            failure_kind: Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR),
        },
    )
    .await;

    let active = load_transport_decode_sticky_escape_states_at(&state.pool, &[account], now)
        .await
        .expect("load active escape state");
    assert_eq!(
        active.get(&account).map(|state| state.until),
        Some(now + ChronoDuration::seconds(299))
    );
    let at_boundary = load_transport_decode_sticky_escape_states_at(
        &state.pool,
        &[account],
        now + ChronoDuration::seconds(299),
    )
    .await
    .expect("load boundary escape state");
    assert!(at_boundary.is_empty(), "now == until must be expired");
}

#[tokio::test]
pub(crate) async fn transport_decode_sticky_escape_requires_two_recent_stream_errors() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let now = parse_rfc3339_utc("2026-07-26T12:00:00Z").expect("fixed now");
    let cases = [("single", 1), ("non-stream", 2), ("old", 3)];
    let mut account_ids = Vec::new();
    for (label, _) in cases {
        let route = format!("https://{label}-stream-escape.example.com/backend-api/codex");
        let account = insert_test_pool_api_key_account_with_options(
            &state,
            &format!("Escape {label}"),
            &format!("sk-escape-{label}"),
            Some(test_required_group_name()),
            Some(&route),
        )
        .await;
        account_ids.push(account);
        if label == "single" {
            seed_account_attempt_at(
                &state.pool,
                AccountAttemptSeed {
                    invoke_id: "single-stream-error",
                    sticky_key: "single-stream-error-key",
                    group_name: test_required_group_name(),
                    upstream_account_id: account,
                    upstream_base_url: &route,
                    occurred_at: &format_utc_iso(now - ChronoDuration::seconds(1)),
                    status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
                    phase: POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED,
                    failure_kind: Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR),
                },
            )
            .await;
        } else if label == "non-stream" {
            for (index, failure_kind) in [
                Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR),
                Some("upstream_http_500"),
            ]
            .into_iter()
            .enumerate()
            {
                seed_account_attempt_at(
                    &state.pool,
                    AccountAttemptSeed {
                        invoke_id: &format!("non-stream-{index}"),
                        sticky_key: &format!("non-stream-key-{index}"),
                        group_name: test_required_group_name(),
                        upstream_account_id: account,
                        upstream_base_url: &route,
                        occurred_at: &format_utc_iso(
                            now - ChronoDuration::seconds(index as i64 + 1),
                        ),
                        status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
                        phase: POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED,
                        failure_kind,
                    },
                )
                .await;
            }
        } else {
            for (index, seconds_ago) in [301_i64, 302_i64].into_iter().enumerate() {
                seed_account_attempt_at(
                    &state.pool,
                    AccountAttemptSeed {
                        invoke_id: &format!("old-{index}"),
                        sticky_key: &format!("old-key-{index}"),
                        group_name: test_required_group_name(),
                        upstream_account_id: account,
                        upstream_base_url: &route,
                        occurred_at: &format_utc_iso(now - ChronoDuration::seconds(seconds_ago)),
                        status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
                        phase: POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED,
                        failure_kind: Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR),
                    },
                )
                .await;
            }
        }
    }
    let states = load_transport_decode_sticky_escape_states_at(&state.pool, &account_ids, now)
        .await
        .expect("load escape states");
    assert!(states.is_empty());
}

#[tokio::test]
pub(crate) async fn resolver_applies_prompt_cache_account_binding_over_sticky_route() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let sticky = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Sticky Source",
        "sk-prompt-cache-sticky-source",
        Some(test_required_group_name()),
        Some("https://sticky-source.example.com/backend-api/codex"),
    )
    .await;
    let bound = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Bound Account",
        "sk-prompt-cache-bound-account",
        Some(test_required_group_name()),
        Some("https://bound-account.example.com/backend-api/codex"),
    )
    .await;
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, sticky, &now_iso, Some(1.0), Some(1.0)).await;
    insert_limit_sample_with_usage(&state.pool, bound, &now_iso, Some(30.0), Some(30.0)).await;
    upsert_sticky_route(&state.pool, "prompt-cache-bound-key", sticky, &now_iso)
        .await
        .expect("upsert sticky source");

    let resolution = resolve_pool_account_for_request_with_binding_constraint(
        &state,
        Some("prompt-cache-bound-key"),
        &[],
        &HashSet::new(),
        Some(&PromptCacheConversationBindingConstraint::UpstreamAccount(
            bound,
        )),
    )
    .await
    .expect("resolve account-bound pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected account-bound account");
    };
    assert_eq!(account.account_id, bound);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::FreshAssignment
    );
}

#[tokio::test]
pub(crate) async fn resolver_prompt_cache_group_binding_reselects_within_group_after_recent_stream_errors()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let bound_group = "prompt-cache-group-escape";
    let unhealthy_route = "https://group-escape-unhealthy.example.com/backend-api/codex";
    let healthy_route = "https://group-escape-healthy.example.com/backend-api/codex";
    let unhealthy = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Group Escape Unhealthy",
        "sk-prompt-cache-group-escape-unhealthy",
        Some(bound_group),
        Some(unhealthy_route),
    )
    .await;
    let healthy = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Group Escape Healthy",
        "sk-prompt-cache-group-escape-healthy",
        Some(bound_group),
        Some(healthy_route),
    )
    .await;
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, unhealthy, &now_iso, Some(1.0), Some(1.0)).await;
    insert_limit_sample_with_usage(&state.pool, healthy, &now_iso, Some(30.0), Some(15.0)).await;
    upsert_sticky_route(
        &state.pool,
        "prompt-cache-group-escape-key",
        unhealthy,
        &now_iso,
    )
    .await
    .expect("seed group-bound sticky route");
    seed_account_upstream_stream_error_attempt(
        &state.pool,
        "group-stream-error-1",
        "group-old-key-a",
        bound_group,
        unhealthy,
        unhealthy_route,
    )
    .await;
    seed_account_upstream_stream_error_attempt(
        &state.pool,
        "group-stream-error-2",
        "group-old-key-b",
        bound_group,
        unhealthy,
        unhealthy_route,
    )
    .await;

    let resolution = resolve_pool_account_for_request_with_binding_constraint(
        &state,
        Some("prompt-cache-group-escape-key"),
        &[],
        &HashSet::new(),
        Some(&PromptCacheConversationBindingConstraint::Group(
            bound_group.to_string(),
        )),
    )
    .await
    .expect("resolve escaped group-bound account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected group-bound routing to reselect within the same group");
    };
    assert_eq!(account.account_id, healthy);
    assert_eq!(account.group_name.as_deref(), Some(bound_group));
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::FreshAssignment
    );
}

#[tokio::test]
pub(crate) async fn resolver_forced_prompt_cache_account_binding_bypasses_target_cut_in_policy() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let sticky = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Sticky Cut In Source",
        "sk-prompt-cache-sticky-cut-in-source",
        Some(test_required_group_name()),
        Some("https://sticky-cut-in-source.example.com/backend-api/codex"),
    )
    .await;
    let bound = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Forced No Cut In",
        "sk-prompt-cache-forced-no-cut-in",
        Some(test_required_group_name()),
        Some("https://forced-no-cut-in.example.com/backend-api/codex"),
    )
    .await;
    let no_cut_in_tag = insert_test_tag(
        &state.pool,
        "prompt-cache-forced-no-cut-in",
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
    sync_account_tag_links(&state.pool, bound, &[no_cut_in_tag.summary.id])
        .await
        .expect("attach no cut-in tag");
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, sticky, &now_iso, Some(1.0), Some(1.0)).await;
    insert_limit_sample_with_usage(&state.pool, bound, &now_iso, Some(20.0), Some(20.0)).await;
    upsert_sticky_route(
        &state.pool,
        "prompt-cache-forced-no-cut-in-key",
        sticky,
        &now_iso,
    )
    .await
    .expect("upsert sticky source");

    let resolution = resolve_pool_account_for_request_with_binding_constraint(
        &state,
        Some("prompt-cache-forced-no-cut-in-key"),
        &[],
        &HashSet::new(),
        Some(&PromptCacheConversationBindingConstraint::UpstreamAccount(
            bound,
        )),
    )
    .await
    .expect("resolve forced account-bound pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected forced account-bound account");
    };
    assert_eq!(account.account_id, bound);
}

#[tokio::test]
pub(crate) async fn resolver_explicit_prompt_cache_account_binding_keeps_operator_override_after_recent_stream_errors()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let bound_group = test_required_group_name();
    let unhealthy_route = "https://explicit-override-unhealthy.example.com/backend-api/codex";
    let healthy_route = "https://explicit-override-healthy.example.com/backend-api/codex";
    let unhealthy = insert_test_pool_api_key_account_with_options(
        &state,
        "Explicit Override Unhealthy",
        "sk-explicit-override-unhealthy",
        Some(bound_group),
        Some(unhealthy_route),
    )
    .await;
    let healthy = insert_test_pool_api_key_account_with_options(
        &state,
        "Explicit Override Healthy",
        "sk-explicit-override-healthy",
        Some(bound_group),
        Some(healthy_route),
    )
    .await;
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, unhealthy, &now_iso, Some(1.0), Some(1.0)).await;
    insert_limit_sample_with_usage(&state.pool, healthy, &now_iso, Some(30.0), Some(15.0)).await;
    upsert_sticky_route(
        &state.pool,
        "prompt-cache-explicit-override-key",
        healthy,
        &now_iso,
    )
    .await
    .expect("seed explicit binding sticky source");
    seed_account_upstream_stream_error_attempt(
        &state.pool,
        "explicit-stream-error-1",
        "explicit-old-key-a",
        bound_group,
        unhealthy,
        unhealthy_route,
    )
    .await;
    seed_account_upstream_stream_error_attempt(
        &state.pool,
        "explicit-stream-error-2",
        "explicit-old-key-b",
        bound_group,
        unhealthy,
        unhealthy_route,
    )
    .await;

    let resolution = resolve_pool_account_for_request_with_binding_constraint(
        &state,
        Some("prompt-cache-explicit-override-key"),
        &[],
        &HashSet::new(),
        Some(&PromptCacheConversationBindingConstraint::UpstreamAccount(
            unhealthy,
        )),
    )
    .await
    .expect("resolve explicit operator override");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected explicit account binding to preserve operator override");
    };
    assert_eq!(account.account_id, unhealthy);
}

#[tokio::test]
pub(crate) async fn resolver_forced_prompt_cache_account_binding_bypasses_source_cut_out_policy() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let sticky = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache No Cut Out Source",
        "sk-prompt-cache-no-cut-out-source",
        Some(test_required_group_name()),
        Some("https://no-cut-out-source.example.com/backend-api/codex"),
    )
    .await;
    let bound = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Forced Cut Out Target",
        "sk-prompt-cache-forced-cut-out-target",
        Some(test_required_group_name()),
        Some("https://forced-cut-out-target.example.com/backend-api/codex"),
    )
    .await;
    let no_cut_out_tag = insert_test_tag(
        &state.pool,
        "prompt-cache-source-no-cut-out",
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
    .expect("insert no cut-out tag");
    sync_account_tag_links(&state.pool, sticky, &[no_cut_out_tag.summary.id])
        .await
        .expect("attach no cut-out tag");
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, sticky, &now_iso, Some(1.0), Some(1.0)).await;
    insert_limit_sample_with_usage(&state.pool, bound, &now_iso, Some(20.0), Some(20.0)).await;
    upsert_sticky_route(
        &state.pool,
        "prompt-cache-forced-cut-out-key",
        sticky,
        &now_iso,
    )
    .await
    .expect("upsert sticky source");

    let resolution = resolve_pool_account_for_request_with_binding_constraint(
        &state,
        Some("prompt-cache-forced-cut-out-key"),
        &[],
        &HashSet::new(),
        Some(&PromptCacheConversationBindingConstraint::UpstreamAccount(
            bound,
        )),
    )
    .await
    .expect("resolve forced account-bound pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected forced account-bound account");
    };
    assert_eq!(account.account_id, bound);
}

#[tokio::test]
pub(crate) async fn resolver_prompt_cache_group_binding_bypasses_source_cut_out_policy() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let sticky_group = "prompt-cache-no-cut-out-sticky-group";
    let bound_group = "prompt-cache-cut-out-bound-group";
    let sticky = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Group No Cut Out Source",
        "sk-prompt-cache-group-no-cut-out-source",
        Some(sticky_group),
        Some("https://group-no-cut-out-source.example.com/backend-api/codex"),
    )
    .await;
    let bound = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Bound Group Cut Out Target",
        "sk-prompt-cache-bound-group-cut-out-target",
        Some(bound_group),
        Some("https://bound-group-cut-out-target.example.com/backend-api/codex"),
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET policy_allow_cut_out = 0 WHERE id = ?1")
        .bind(sticky)
        .execute(&state.pool)
        .await
        .expect("set source no cut-out policy");
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, sticky, &now_iso, Some(1.0), Some(1.0)).await;
    insert_limit_sample_with_usage(&state.pool, bound, &now_iso, Some(20.0), Some(20.0)).await;
    upsert_sticky_route(
        &state.pool,
        "prompt-cache-group-source-cut-out-key",
        sticky,
        &now_iso,
    )
    .await
    .expect("upsert sticky source");

    let resolution = resolve_pool_account_for_request_with_binding_constraint(
        &state,
        Some("prompt-cache-group-source-cut-out-key"),
        &[],
        &HashSet::new(),
        Some(&PromptCacheConversationBindingConstraint::Group(
            bound_group.to_string(),
        )),
    )
    .await
    .expect("resolve group-bound cut-out pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected group-bound account to cut out from sticky source");
    };
    assert_eq!(account.account_id, bound);
    assert_eq!(account.group_name.as_deref(), Some(bound_group));
}

#[tokio::test]
pub(crate) async fn resolver_blocks_cut_out_when_sticky_route_key_is_excluded_by_failover() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let sticky_route = "https://sticky-excluded-no-cut-out.example.com/backend-api/codex";
    let fallback_route = "https://fallback-excluded-no-cut-out.example.com/backend-api/codex";
    let sticky = insert_test_pool_api_key_account_with_options(
        &state,
        "Sticky Excluded No Cut Out Source",
        "sk-sticky-excluded-no-cut-out-source",
        Some(test_required_group_name()),
        Some(sticky_route),
    )
    .await;
    let fallback = insert_test_pool_api_key_account_with_options(
        &state,
        "Fallback Excluded No Cut Out Target",
        "sk-fallback-excluded-no-cut-out-target",
        Some(test_required_group_name()),
        Some(fallback_route),
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET policy_allow_cut_out = 0 WHERE id = ?1")
        .bind(sticky)
        .execute(&state.pool)
        .await
        .expect("set sticky source no cut-out policy");
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, sticky, &now_iso, Some(1.0), Some(1.0)).await;
    insert_limit_sample_with_usage(&state.pool, fallback, &now_iso, Some(20.0), Some(20.0)).await;
    upsert_sticky_route(
        &state.pool,
        "sticky-excluded-no-cut-out-key",
        sticky,
        &now_iso,
    )
    .await
    .expect("upsert sticky route");
    let excluded_upstream_route_keys = HashSet::from([canonical_pool_upstream_route_key(
        &Url::parse(sticky_route).expect("valid sticky route"),
    )]);

    let resolution = resolve_pool_account_for_request(
        &state,
        Some("sticky-excluded-no-cut-out-key"),
        &[],
        &excluded_upstream_route_keys,
    )
    .await
    .expect("resolve excluded sticky pool account");
    let PoolAccountResolution::AssignedBlocked(blocked) = resolution else {
        panic!("expected excluded no-cut-out sticky source to block automatic cut-out");
    };
    assert_eq!(blocked.account.account_id, sticky);
    assert!(
        blocked.message.contains("routing policy forbids it"),
        "unexpected blocked message: {}",
        blocked.message
    );
}

#[tokio::test]
pub(crate) async fn resolver_blocks_cut_out_when_sticky_source_is_hard_blocked() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let sticky = insert_test_pool_api_key_account_with_options(
        &state,
        "Sticky Hard Blocked No Cut Out Source",
        "sk-sticky-hard-blocked-no-cut-out-source",
        Some(test_required_group_name()),
        Some("https://sticky-hard-blocked-no-cut-out.example.com/backend-api/codex"),
    )
    .await;
    let fallback = insert_test_pool_api_key_account_with_options(
        &state,
        "Fallback Hard Blocked No Cut Out Target",
        "sk-fallback-hard-blocked-no-cut-out-target",
        Some(test_required_group_name()),
        Some("https://fallback-hard-blocked-no-cut-out.example.com/backend-api/codex"),
    )
    .await;
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET policy_allow_cut_out = 0,
            status = 'error'
        WHERE id = ?1
        "#,
    )
    .bind(sticky)
    .execute(&state.pool)
    .await
    .expect("set sticky source no cut-out hard block");
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, fallback, &now_iso, Some(20.0), Some(20.0)).await;
    upsert_sticky_route(
        &state.pool,
        "sticky-hard-blocked-no-cut-out-key",
        sticky,
        &now_iso,
    )
    .await
    .expect("upsert hard-blocked sticky route");

    let resolution = resolve_pool_account_for_request(
        &state,
        Some("sticky-hard-blocked-no-cut-out-key"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve hard-blocked sticky pool account");
    let message = match resolution {
        PoolAccountResolution::AssignedBlocked(blocked) => {
            assert_eq!(blocked.account.account_id, sticky);
            blocked.message
        }
        PoolAccountResolution::BlockedByPolicy(message) => message,
        PoolAccountResolution::Resolved(account) => {
            panic!(
                "hard-blocked no-cut-out sticky source cut out to account {}",
                account.account_id
            )
        }
        other => panic!("expected no-cut-out sticky source to block, got {other:?}"),
    };
    assert!(
        message.contains("routing policy forbids it"),
        "unexpected blocked message: {message}",
    );
}

#[tokio::test]
pub(crate) async fn resolver_forced_prompt_cache_account_binding_reuses_blocked_sticky_owner() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let bound = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Forced Blocked Sticky Owner",
        "sk-prompt-cache-forced-blocked-sticky-owner",
        Some(test_required_group_name()),
        Some("https://forced-blocked-sticky-owner.example.com/backend-api/codex"),
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET policy_priority_tier = 'no_new' WHERE id = ?1")
        .bind(bound)
        .execute(&state.pool)
        .await
        .expect("set block new conversations");
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, bound, &now_iso, Some(20.0), Some(20.0)).await;
    upsert_sticky_route(
        &state.pool,
        "prompt-cache-forced-blocked-sticky-owner-key",
        bound,
        &now_iso,
    )
    .await
    .expect("upsert forced binding sticky owner");

    let resolution = resolve_pool_account_for_request_with_binding_constraint(
        &state,
        Some("prompt-cache-forced-blocked-sticky-owner-key"),
        &[],
        &HashSet::new(),
        Some(&PromptCacheConversationBindingConstraint::UpstreamAccount(
            bound,
        )),
    )
    .await
    .expect("resolve forced blocked sticky owner");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected forced blocked sticky owner reuse");
    };
    assert_eq!(account.account_id, bound);
}

#[tokio::test]
pub(crate) async fn resolver_forced_prompt_cache_account_binding_keeps_concurrency_limit() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let sticky = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Concurrency Source",
        "sk-prompt-cache-concurrency-source",
        Some(test_required_group_name()),
        Some("https://concurrency-source.example.com/backend-api/codex"),
    )
    .await;
    let bound = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Forced Concurrency Target",
        "sk-prompt-cache-forced-concurrency-target",
        Some(test_required_group_name()),
        Some("https://forced-concurrency-target.example.com/backend-api/codex"),
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET policy_concurrency_limit = 1 WHERE id = ?1")
        .bind(bound)
        .execute(&state.pool)
        .await
        .expect("set account concurrency limit");
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, sticky, &now_iso, Some(1.0), Some(1.0)).await;
    insert_limit_sample_with_usage(&state.pool, bound, &now_iso, Some(20.0), Some(20.0)).await;
    upsert_sticky_route(
        &state.pool,
        "prompt-cache-concurrency-source-key",
        sticky,
        &now_iso,
    )
    .await
    .expect("upsert sticky source");
    upsert_sticky_route(
        &state.pool,
        "prompt-cache-concurrency-active-key",
        bound,
        &now_iso,
    )
    .await
    .expect("upsert active target sticky");

    let resolution = resolve_pool_account_for_request_with_binding_constraint(
        &state,
        Some("prompt-cache-concurrency-source-key"),
        &[],
        &HashSet::new(),
        Some(&PromptCacheConversationBindingConstraint::UpstreamAccount(
            bound,
        )),
    )
    .await
    .expect("resolve forced account-bound pool account");
    assert!(matches!(resolution, PoolAccountResolution::Unavailable));
}

#[tokio::test]
pub(crate) async fn resolver_forced_prompt_cache_account_binding_keeps_concurrency_limit_after_sticky_upsert()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let bound = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Forced Sticky Concurrency Target",
        "sk-prompt-cache-forced-sticky-concurrency-target",
        Some(test_required_group_name()),
        Some("https://forced-sticky-concurrency-target.example.com/backend-api/codex"),
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET policy_concurrency_limit = 1 WHERE id = ?1")
        .bind(bound)
        .execute(&state.pool)
        .await
        .expect("set account concurrency limit");
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, bound, &now_iso, Some(20.0), Some(20.0)).await;
    upsert_sticky_route(
        &state.pool,
        "prompt-cache-forced-sticky-concurrency-key",
        bound,
        &now_iso,
    )
    .await
    .expect("upsert forced binding sticky");
    upsert_sticky_route(
        &state.pool,
        "prompt-cache-forced-sticky-concurrency-other-key",
        bound,
        &now_iso,
    )
    .await
    .expect("upsert other active target sticky");

    let resolution = resolve_pool_account_for_request_with_binding_constraint(
        &state,
        Some("prompt-cache-forced-sticky-concurrency-key"),
        &[],
        &HashSet::new(),
        Some(&PromptCacheConversationBindingConstraint::UpstreamAccount(
            bound,
        )),
    )
    .await
    .expect("resolve forced account-bound pool account");
    assert!(matches!(resolution, PoolAccountResolution::Unavailable));
}

#[tokio::test]
pub(crate) async fn resolver_prompt_cache_group_binding_does_not_bypass_cut_in_policy() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let sticky_group = "prompt-cache-group-cut-in-source";
    let bound_group = "prompt-cache-group-cut-in-bound";
    let sticky = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Group Sticky Source",
        "sk-prompt-cache-group-sticky-source",
        Some(sticky_group),
        Some("https://group-sticky-source.example.com/backend-api/codex"),
    )
    .await;
    let bound = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Group No Cut In",
        "sk-prompt-cache-group-no-cut-in",
        Some(bound_group),
        Some("https://group-no-cut-in.example.com/backend-api/codex"),
    )
    .await;
    let no_cut_in_tag = insert_test_tag(
        &state.pool,
        "prompt-cache-group-no-cut-in",
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
    sync_account_tag_links(&state.pool, bound, &[no_cut_in_tag.summary.id])
        .await
        .expect("attach no cut-in tag");
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, sticky, &now_iso, Some(1.0), Some(1.0)).await;
    insert_limit_sample_with_usage(&state.pool, bound, &now_iso, Some(20.0), Some(20.0)).await;
    upsert_sticky_route(
        &state.pool,
        "prompt-cache-group-cut-in-key",
        sticky,
        &now_iso,
    )
    .await
    .expect("upsert sticky source");

    let resolution = resolve_pool_account_for_request_with_binding_constraint(
        &state,
        Some("prompt-cache-group-cut-in-key"),
        &[],
        &HashSet::new(),
        Some(&PromptCacheConversationBindingConstraint::Group(
            bound_group.to_string(),
        )),
    )
    .await
    .expect("resolve group-bound pool account");
    assert!(matches!(resolution, PoolAccountResolution::Unavailable));
}

#[tokio::test]
pub(crate) async fn resolver_prompt_cache_group_binding_bypasses_requested_model_filter() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let bound_group = "prompt-cache-group-model-bypass";
    let bound = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Group Model Bypass",
        "sk-prompt-cache-group-model-bypass",
        Some(bound_group),
        Some("https://group-model-bypass.example.com/backend-api/codex"),
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET policy_available_models_json = '[\"gpt-4o\"]' WHERE id = ?1")
        .bind(bound)
        .execute(&state.pool)
        .await
        .expect("make bound account model-constrained");
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, bound, &now_iso, Some(20.0), Some(20.0)).await;

    let resolution = resolve_pool_account_for_request_with_binding_constraint_and_model(
        &state,
        Some("prompt-cache-group-model-bypass-key"),
        Some("gpt-5.5"),
        &[],
        &HashSet::new(),
        Some(&PromptCacheConversationBindingConstraint::Group(
            bound_group.to_string(),
        )),
    )
    .await
    .expect("resolve group-bound pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected explicit group binding to bypass requested model filter");
    };
    assert_eq!(account.account_id, bound);
}

pub(crate) async fn seed_route_binding_attempt(
    pool: &SqlitePool,
    invoke_id: &str,
    upstream_base_url: &str,
    proxy_binding_key_snapshot: &str,
    status: &str,
    failure_kind: Option<&str>,
) {
    let upstream_route_key = canonical_pool_upstream_route_key(
        &Url::parse(upstream_base_url).expect("valid upstream route"),
    );
    let now_iso = format_utc_iso(Utc::now());
    let phase = if status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS {
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED
    } else {
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED
    };
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            invoke_id,
            occurred_at,
            endpoint,
            route_mode,
            sticky_key,
            group_name_snapshot,
            proxy_binding_key_snapshot,
            upstream_account_id,
            upstream_route_key,
            attempt_index,
            distinct_account_index,
            same_account_retry_index,
            requester_ip,
            started_at,
            finished_at,
            status,
            phase,
            http_status,
            error_message,
            failure_kind,
            created_at
        )
        VALUES (
            ?1, ?2, '/v1/responses', ?3, NULL, ?4, ?5, 41, ?6,
            1, 1, 0, '203.0.113.10', ?2, ?2, ?7, ?8, ?9, ?10, ?11, datetime('now')
        )
        "#,
    )
    .bind(invoke_id)
    .bind(&now_iso)
    .bind(INVOCATION_ROUTE_MODE_POOL)
    .bind(test_required_group_name())
    .bind(proxy_binding_key_snapshot)
    .bind(upstream_route_key)
    .bind(status)
    .bind(phase)
    .bind((status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS).then_some(200_i64))
    .bind((status != POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS).then_some("test failure"))
    .bind(failure_kind)
    .execute(pool)
    .await
    .expect("seed route binding attempt");
}

pub(crate) async fn seed_account_upstream_stream_error_attempt(
    pool: &SqlitePool,
    invoke_id: &str,
    sticky_key: &str,
    group_name: &str,
    upstream_account_id: i64,
    upstream_base_url: &str,
) {
    seed_account_attempt_at(
        pool,
        AccountAttemptSeed {
            invoke_id,
            sticky_key,
            group_name,
            upstream_account_id,
            upstream_base_url,
            occurred_at: &format_utc_iso(Utc::now()),
            status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
            phase: POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED,
            failure_kind: Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR),
        },
    )
    .await;
}

pub(crate) struct AccountAttemptSeed<'a> {
    pub(crate) invoke_id: &'a str,
    pub(crate) sticky_key: &'a str,
    pub(crate) group_name: &'a str,
    pub(crate) upstream_account_id: i64,
    pub(crate) upstream_base_url: &'a str,
    pub(crate) occurred_at: &'a str,
    pub(crate) status: &'a str,
    pub(crate) phase: &'a str,
    pub(crate) failure_kind: Option<&'a str>,
}

pub(crate) async fn seed_account_attempt_at(pool: &SqlitePool, seed: AccountAttemptSeed<'_>) {
    let upstream_route_key = canonical_pool_upstream_route_key(
        &Url::parse(seed.upstream_base_url).expect("valid upstream route"),
    );
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            invoke_id,
            occurred_at,
            endpoint,
            route_mode,
            sticky_key,
            group_name_snapshot,
            proxy_binding_key_snapshot,
            upstream_account_id,
            upstream_route_key,
            attempt_index,
            distinct_account_index,
            same_account_retry_index,
            requester_ip,
            started_at,
            finished_at,
            status,
            phase,
            http_status,
            error_message,
            failure_kind,
            created_at
        )
        VALUES (
            ?1, ?2, '/v1/responses', ?3, ?4, ?5, ?6, ?7, ?8,
            1, 1, 0, '203.0.113.11', ?2, ?2, ?9, ?10, NULL, ?11, ?12, datetime('now')
        )
        "#,
    )
    .bind(seed.invoke_id)
    .bind(seed.occurred_at)
    .bind(INVOCATION_ROUTE_MODE_POOL)
    .bind(seed.sticky_key)
    .bind(seed.group_name)
    .bind(FORWARD_PROXY_DIRECT_KEY)
    .bind(seed.upstream_account_id)
    .bind(upstream_route_key)
    .bind(seed.status)
    .bind(seed.phase)
    .bind((seed.failure_kind.is_some()).then_some("synthetic test failure"))
    .bind(seed.failure_kind)
    .execute(pool)
    .await
    .expect("seed upstream stream error attempt");
}
