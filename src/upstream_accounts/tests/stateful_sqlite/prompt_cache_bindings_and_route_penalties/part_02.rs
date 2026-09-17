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

use super::*;
