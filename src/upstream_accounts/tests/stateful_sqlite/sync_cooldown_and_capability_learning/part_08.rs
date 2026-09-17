#[tokio::test]
pub(crate) async fn resolver_skips_persisted_snapshot_exhausted_account_before_routing() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let exhausted = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Exhausted Candidate",
        "exhausted-candidate@example.com",
        "org_exhausted_candidate",
        "user_exhausted_candidate",
    )
    .await;
    let available = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Available Candidate",
        "available-candidate@example.com",
        "org_available_candidate",
        "user_available_candidate",
    )
    .await;
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, exhausted, &now_iso, Some(100.0), Some(20.0)).await;
    insert_limit_sample_with_usage(&state.pool, available, &now_iso, Some(42.0), Some(10.0)).await;

    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected resolver to pick an available account");
    };
    assert_eq!(account.account_id, available);
}

#[tokio::test]
pub(crate) async fn resolver_reuses_sticky_snapshot_exhausted_account_until_conversation_gets_429()
{
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let exhausted = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Sticky Snapshot Exhausted",
        "sticky-snapshot-exhausted@example.com",
        "org_sticky_snapshot_exhausted",
        "user_sticky_snapshot_exhausted",
    )
    .await;
    let available = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Sticky Snapshot Available",
        "sticky-snapshot-available@example.com",
        "org_sticky_snapshot_available",
        "user_sticky_snapshot_available",
    )
    .await;
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, exhausted, &now_iso, Some(100.0), Some(20.0)).await;
    insert_limit_sample_with_usage(&state.pool, available, &now_iso, Some(42.0), Some(10.0)).await;
    upsert_sticky_route(
        &state.pool,
        "sticky-snapshot-exhausted",
        exhausted,
        &now_iso,
    )
    .await
    .expect("seed sticky route");

    let resolution = resolve_pool_account_for_request(
        &state,
        Some("sticky-snapshot-exhausted"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve sticky snapshot exhausted account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected sticky exhausted account to remain reusable, got {resolution:?}");
    };
    assert_eq!(account.account_id, exhausted);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::StickyReuse
    );
}

#[tokio::test]
pub(crate) async fn resolver_preserves_sticky_record_but_rotates_after_auth_hard_failure() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let failed = insert_test_pool_api_key_account_with_options(
        &state,
        "Sticky Auth Failed",
        "sk-sticky-auth-failed",
        Some("sticky-auth-rotation"),
        Some("https://sticky-auth-failed.example.com/backend-api/codex"),
    )
    .await;
    let available = insert_test_pool_api_key_account_with_options(
        &state,
        "Sticky Auth Replacement",
        "sk-sticky-auth-replacement",
        Some("sticky-auth-rotation"),
        Some("https://sticky-auth-replacement.example.com/backend-api/codex"),
    )
    .await;
    let now_iso = format_utc_iso(Utc::now());
    upsert_sticky_route(&state.pool, "sticky-auth-failed", failed, &now_iso)
        .await
        .expect("seed auth sticky route");

    record_pool_route_http_failure(
        &state.pool,
        failed,
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
        false,
        Some("sticky-auth-failed"),
        StatusCode::UNAUTHORIZED,
        "pool upstream responded with 401: invalid api key",
        Some("invk_auth_failed"),
    )
    .await
    .expect("record auth hard failure");

    assert_eq!(
        load_sticky_route(&state.pool, "sticky-auth-failed")
            .await
            .expect("load preserved sticky route")
            .map(|route| route.account_id),
        Some(failed),
    );

    let resolution =
        resolve_pool_account_for_request(&state, Some("sticky-auth-failed"), &[], &HashSet::new())
            .await
            .expect("resolve after auth hard failure");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected resolver to rotate to another available account, got {resolution:?}");
    };
    assert_eq!(account.account_id, available);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::FreshAssignment
    );
}

#[tokio::test]
pub(crate) async fn resolver_prefers_primary_priority_before_normal_and_fallback() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let fallback_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Fallback Priority Candidate",
        "sk-priority-fallback",
        Some("routing-priority"),
        Some("https://routing-fallback.example.com/backend-api/codex"),
    )
    .await;
    let normal_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Normal Priority Candidate",
        "sk-priority-normal",
        Some("routing-priority"),
        Some("https://routing-normal.example.com/backend-api/codex"),
    )
    .await;
    let primary_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Primary Priority Candidate",
        "sk-priority-primary",
        Some("routing-priority"),
        Some("https://routing-primary.example.com/backend-api/codex"),
    )
    .await;

    let mut fallback_rule = test_tag_routing_rule();
    fallback_rule.priority_tier = TagPriorityTier::Fallback;
    let fallback_tag = insert_test_tag(&state.pool, "fallback-priority", &fallback_rule)
        .await
        .expect("insert fallback tag");
    let normal_tag = insert_test_tag(&state.pool, "normal-priority", &test_tag_routing_rule())
        .await
        .expect("insert normal tag");
    let mut primary_rule = test_tag_routing_rule();
    primary_rule.priority_tier = TagPriorityTier::Primary;
    let primary_tag = insert_test_tag(&state.pool, "primary-priority", &primary_rule)
        .await
        .expect("insert primary tag");
    sync_account_tag_links(&state.pool, fallback_account_id, &[fallback_tag.summary.id])
        .await
        .expect("attach fallback tag");
    sync_account_tag_links(&state.pool, normal_account_id, &[normal_tag.summary.id])
        .await
        .expect("attach normal tag");
    sync_account_tag_links(&state.pool, primary_account_id, &[primary_tag.summary.id])
        .await
        .expect("attach primary tag");

    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(
        &state.pool,
        fallback_account_id,
        &now_iso,
        Some(1.0),
        Some(1.0),
    )
    .await;
    insert_limit_sample_with_usage(
        &state.pool,
        normal_account_id,
        &now_iso,
        Some(10.0),
        Some(1.0),
    )
    .await;
    insert_limit_sample_with_usage(
        &state.pool,
        primary_account_id,
        &now_iso,
        Some(35.0),
        Some(1.0),
    )
    .await;

    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected resolver to pick a prioritized account");
    };
    assert_eq!(account.account_id, primary_account_id);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::FreshAssignment
    );
}

#[tokio::test]
pub(crate) async fn resolver_proactively_hands_off_fallback_sticky_to_higher_priority_account() {
    let _priority_handoff_guard = crate::upstream_accounts::priority_handoff_test_guard();
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let fallback_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Fallback Sticky Handoff Source",
        "sk-fallback-sticky-handoff-source",
        Some("fallback-sticky-handoff"),
        Some("https://fallback-sticky-handoff-source.example.com/backend-api/codex"),
    )
    .await;
    let primary_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Primary Sticky Handoff Target",
        "sk-primary-sticky-handoff-target",
        Some("fallback-sticky-handoff"),
        Some("https://primary-sticky-handoff-target.example.com/backend-api/codex"),
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET policy_priority_tier = ?2 WHERE id = ?1")
        .bind(fallback_account_id)
        .bind(TagPriorityTier::Fallback.as_str())
        .execute(&state.pool)
        .await
        .expect("set fallback sticky priority");
    sqlx::query("UPDATE pool_upstream_accounts SET policy_priority_tier = ?2 WHERE id = ?1")
        .bind(primary_account_id)
        .bind(TagPriorityTier::Primary.as_str())
        .execute(&state.pool)
        .await
        .expect("set primary handoff priority");

    let sticky_key = "fallback-sticky-handoff";
    let now_iso = format_utc_iso(Utc::now());
    upsert_sticky_route(&state.pool, sticky_key, fallback_account_id, &now_iso)
        .await
        .expect("seed fallback sticky route");

    let resolution = resolve_pool_account_for_request_with_binding_constraint_and_model(
        &state,
        Some(sticky_key),
        Some("gpt-fallback-sticky-handoff"),
        &[],
        &HashSet::new(),
        None,
    )
    .await
    .expect("resolve fallback sticky handoff");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected higher priority account to receive the request");
    };
    assert_eq!(account.account_id, primary_account_id);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::PriorityHandoff
    );
    assert_eq!(
        load_sticky_route(&state.pool, sticky_key)
            .await
            .expect("load sticky route before success")
            .map(|route| route.account_id),
        Some(fallback_account_id),
    );

    assert_sticky_reused_without_model(&state, sticky_key, fallback_account_id).await;

    record_pool_route_success_with_affinity_generation(
        &state.pool,
        account.account_id,
        Utc::now(),
        Some(sticky_key),
        None,
        None,
        account.sticky_affinity_generation,
    )
    .await
    .expect("record successful handoff");
    assert_eq!(
        load_sticky_route(&state.pool, sticky_key)
            .await
            .expect("load sticky route after success")
            .map(|route| route.account_id),
        Some(primary_account_id),
    );
}

async fn assert_sticky_reused_without_model(
    state: &AppState,
    sticky_key: &str,
    fallback_account_id: i64,
) {
    let resolution = resolve_pool_account_for_request_with_binding_constraint_and_model(
        state,
        Some(sticky_key),
        None,
        &[],
        &HashSet::new(),
        None,
    )
    .await
    .expect("resolve fallback sticky without model");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected fallback sticky source without a request model");
    };
    assert_eq!(account.account_id, fallback_account_id);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::StickyReuse
    );
}

#[tokio::test]
pub(crate) async fn resolver_bypasses_busy_priority_handoff_for_fresh_assignment() {
    let _priority_handoff_guard = crate::upstream_accounts::priority_handoff_test_guard();
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let target_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Fresh Assignment Handoff Target",
        "sk-fresh-assignment-handoff-target",
        Some("fresh-assignment-handoff"),
        Some("https://fresh-assignment-handoff-target.example.com/backend-api/codex"),
    )
    .await;
    let alternate_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Fresh Assignment Handoff Alternate",
        "sk-fresh-assignment-handoff-alternate",
        Some("fresh-assignment-handoff"),
        Some("https://fresh-assignment-handoff-alternate.example.com/backend-api/codex"),
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET policy_priority_tier = ?2 WHERE id = ?1")
        .bind(target_account_id)
        .bind(TagPriorityTier::Primary.as_str())
        .execute(&state.pool)
        .await
        .expect("set fresh assignment target priority");

    let (decision, held_permit) =
        admit_priority_handoff(target_account_id, Some("gpt-fresh-assignment-handoff"));
    assert!(matches!(
        decision,
        PriorityHandoffAdmissionDecision::Admitted { .. }
    ));
    assert!(held_permit.is_some());

    let resolution = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        None,
        Some("gpt-fresh-assignment-handoff"),
        &[],
        &HashSet::new(),
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        None,
    )
    .await
    .expect("resolve fresh assignment around busy handoff");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected alternate account after busy handoff, got {resolution:?}");
    };
    assert_eq!(account.account_id, alternate_account_id);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::PriorityHandoff
    );
    let audit = account
        .routing_selection_audit
        .expect("fresh assignment should carry handoff audit");
    assert_eq!(
        audit
            .handoff_admission
            .as_ref()
            .expect("busy handoff decision should be audited")
            .decision,
        "admitted"
    );
    assert_eq!(audit.excluded_candidates[0].account_id, target_account_id);
    let admitted_generation = audit
        .handoff_admission
        .as_ref()
        .expect("alternate handoff should carry admission generation")
        .generation;
    assert_eq!(
        complete_priority_handoff_for_request(
            alternate_account_id,
            Some("gpt-fresh-assignment-handoff"),
            Some(admitted_generation),
            true,
            false,
        ),
        Some(PRIORITY_HANDOFF_RECOVERY_PROGRESS_REASON)
    );
    assert_eq!(
        priority_handoff_admission_snapshot(
            alternate_account_id,
            Some("gpt-fresh-assignment-handoff")
        ),
        ("verifying".to_string(), 1)
    );
    drop(held_permit);
}

#[tokio::test]
pub(crate) async fn resolver_request_driven_priority_recovery_precedes_healthy_lower_priority_winner()
 {
    let _priority_handoff_guard = crate::upstream_accounts::priority_handoff_test_guard();
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let recovery_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Degraded Primary Recovery Target",
        "sk-request-recovery-target",
        Some("request-driven-recovery"),
        Some("https://request-recovery-target.example.com/backend-api/codex"),
    )
    .await;
    let healthy_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Healthy Normal Winner",
        "sk-request-recovery-healthy",
        Some("request-driven-recovery"),
        Some("https://request-recovery-healthy.example.com/backend-api/codex"),
    )
    .await;
    for (account_id, tier) in [
        (recovery_account_id, TagPriorityTier::Primary),
        (healthy_account_id, TagPriorityTier::Normal),
    ] {
        sqlx::query("UPDATE pool_upstream_accounts SET policy_priority_tier = ?2 WHERE id = ?1")
            .bind(account_id)
            .bind(tier.as_str())
            .execute(&state.pool)
            .await
            .expect("set request recovery priority");
    }
    let requested_model = "gpt-request-driven-recovery";
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        "INSERT INTO pool_upstream_account_model_routes (account_id, model, state, priority, consecutive_failures, changed_at, last_seen_at, last_failure_at, last_failure_kind, last_failure_message) VALUES (?1, ?2, 'degraded', 'demoted', 1, ?3, ?3, ?3, 'model_unavailable', 'model unavailable')",
    )
    .bind(recovery_account_id)
    .bind(requested_model)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .expect("seed degraded recovery model route");

    let resolution = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        None,
        Some(requested_model),
        &[],
        &HashSet::new(),
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        None,
    )
    .await
    .expect("resolve request-driven recovery");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected recovery target to be admitted, got {resolution:?}");
    };
    assert_eq!(account.account_id, recovery_account_id);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::PriorityHandoff
    );
    let audit = account
        .routing_selection_audit
        .expect("recovery admission should carry an audit");
    assert_eq!(audit.winner_reason_code, "requestDrivenRecoveryAdmission");
    let admission = audit
        .handoff_admission
        .expect("recovery admission should be recorded");
    assert_eq!(admission.trigger.as_deref(), Some("modelRouteRecovery"));
    assert_eq!(admission.verification_success_count, 0);
    assert_ne!(account.account_id, healthy_account_id);
}

#[tokio::test]
pub(crate) async fn resolver_admits_first_untracked_priority_fresh_assignment() {
    let _priority_handoff_guard = crate::upstream_accounts::priority_handoff_test_guard();
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let target_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Untracked Fresh Assignment Handoff Target",
        "sk-untracked-fresh-assignment-handoff-target",
        Some("untracked-fresh-assignment-handoff"),
        Some("https://untracked-fresh-assignment-handoff-target.example.com/backend-api/codex"),
    )
    .await;
    let alternate_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Untracked Fresh Assignment Handoff Alternate",
        "sk-untracked-fresh-assignment-handoff-alternate",
        Some("untracked-fresh-assignment-handoff"),
        Some("https://untracked-fresh-assignment-handoff-alternate.example.com/backend-api/codex"),
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET policy_priority_tier = ?2 WHERE id = ?1")
        .bind(target_account_id)
        .bind(TagPriorityTier::Primary.as_str())
        .execute(&state.pool)
        .await
        .expect("set untracked fresh assignment target priority");

    let resolution = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        None,
        Some("gpt-untracked-fresh-assignment-handoff"),
        &[],
        &HashSet::new(),
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        None,
    )
    .await
    .expect("resolve first untracked priority assignment");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected first untracked priority assignment, got {resolution:?}");
    };
    assert_eq!(account.account_id, target_account_id);
    assert_ne!(account.account_id, alternate_account_id);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::PriorityHandoff
    );
    assert_eq!(
        account
            .routing_selection_audit
            .as_ref()
            .and_then(|audit| audit.handoff_admission.as_ref())
            .map(|admission| admission.decision.as_str()),
        Some("admitted")
    );
}

#[tokio::test]
pub(crate) async fn resolver_keeps_fallback_sticky_when_no_higher_priority_candidate_exists() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let sticky_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Fallback Sticky Source",
        "sk-fallback-sticky-source",
        Some("fallback-sticky-only"),
        Some("https://fallback-sticky-source.example.com/backend-api/codex"),
    )
    .await;
    let peer_fallback_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Fallback Sticky Peer",
        "sk-fallback-sticky-peer",
        Some("fallback-sticky-only"),
        Some("https://fallback-sticky-peer.example.com/backend-api/codex"),
    )
    .await;
    for account_id in [sticky_account_id, peer_fallback_account_id] {
        sqlx::query("UPDATE pool_upstream_accounts SET policy_priority_tier = ?2 WHERE id = ?1")
            .bind(account_id)
            .bind(TagPriorityTier::Fallback.as_str())
            .execute(&state.pool)
            .await
            .expect("set fallback-only priority");
    }

    let sticky_key = "fallback-sticky-only";
    upsert_sticky_route(
        &state.pool,
        sticky_key,
        sticky_account_id,
        &format_utc_iso(Utc::now()),
    )
    .await
    .expect("seed fallback-only sticky route");

    let resolution =
        resolve_pool_account_for_request(&state, Some(sticky_key), &[], &HashSet::new())
            .await
            .expect("resolve fallback-only sticky route");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected fallback sticky account to remain reusable");
    };
    assert_eq!(account.account_id, sticky_account_id);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::StickyReuse
    );
}

#[tokio::test]
pub(crate) async fn resolver_preserves_same_tier_fallback_penalty_failover_during_proactive_handoff()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let sticky_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Penalized Fallback Sticky Source",
        "sk-penalized-fallback-sticky-source",
        Some("fallback-sticky-penalty"),
        Some("https://penalized-fallback-sticky-source.example.com/backend-api/codex"),
    )
    .await;
    let peer_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Healthy Fallback Peer",
        "sk-healthy-fallback-peer",
        Some("fallback-sticky-penalty"),
        Some("https://healthy-fallback-peer.example.com/backend-api/codex"),
    )
    .await;
    for account_id in [sticky_account_id, peer_account_id] {
        sqlx::query("UPDATE pool_upstream_accounts SET policy_priority_tier = ?2 WHERE id = ?1")
            .bind(account_id)
            .bind(TagPriorityTier::Fallback.as_str())
            .execute(&state.pool)
            .await
            .expect("set fallback penalty priority");
    }

    let requested_model = "gpt-fallback-penalty";
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        "INSERT INTO pool_upstream_account_model_routes (account_id, model, state, priority, consecutive_failures, changed_at, last_seen_at, last_failure_at, last_failure_kind, last_failure_message) VALUES (?1, ?2, 'degraded', 'demoted', 1, ?3, ?3, ?3, 'model_unavailable', 'model unavailable')",
    )
    .bind(sticky_account_id)
    .bind(requested_model)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .expect("seed fallback model penalty");
    let sticky_key = "fallback-sticky-penalty";
    upsert_sticky_route(&state.pool, sticky_key, sticky_account_id, &now_iso)
        .await
        .expect("seed penalized fallback sticky route");

    let resolution = resolve_pool_account_for_request_with_binding_constraint_and_model(
        &state,
        Some(sticky_key),
        Some(requested_model),
        &[],
        &HashSet::new(),
        None,
    )
    .await
    .expect("resolve fallback penalty failover");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected healthy same-tier fallback peer to win");
    };
    assert_eq!(account.account_id, peer_account_id);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::FreshAssignment
    );
}

#[tokio::test]
pub(crate) async fn resolver_allows_fallback_failover_when_sticky_source_is_unusable() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let sticky_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Unusable Fallback Sticky Source",
        "sk-unusable-fallback-sticky-source",
        Some("fallback-sticky-unusable"),
        Some("https://unusable-fallback-sticky-source.example.com/backend-api/codex"),
    )
    .await;
    let fallback_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Fallback Failover Candidate",
        "sk-fallback-failover-candidate",
        Some("fallback-sticky-unusable"),
        Some("https://fallback-failover-candidate.example.com/backend-api/codex"),
    )
    .await;
    for account_id in [sticky_account_id, fallback_account_id] {
        sqlx::query("UPDATE pool_upstream_accounts SET policy_priority_tier = ?2 WHERE id = ?1")
            .bind(account_id)
            .bind(TagPriorityTier::Fallback.as_str())
            .execute(&state.pool)
            .await
            .expect("set fallback failover priority");
    }
    sqlx::query("UPDATE pool_upstream_accounts SET status = ?2 WHERE id = ?1")
        .bind(sticky_account_id)
        .bind(UPSTREAM_ACCOUNT_STATUS_ERROR)
        .execute(&state.pool)
        .await
        .expect("make sticky fallback unusable");

    let sticky_key = "fallback-sticky-unusable";
    upsert_sticky_route(
        &state.pool,
        sticky_key,
        sticky_account_id,
        &format_utc_iso(Utc::now()),
    )
    .await
    .expect("seed unusable fallback sticky route");

    let resolution =
        resolve_pool_account_for_request(&state, Some(sticky_key), &[], &HashSet::new())
            .await
            .expect("resolve fallback failover");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected usable fallback candidate after sticky source became unusable");
    };
    assert_eq!(account.account_id, fallback_account_id);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::FreshAssignment
    );
}

#[tokio::test]
pub(crate) async fn resolver_keeps_higher_priority_soft_degraded_candidate_ahead_of_lower_priority_ready_account()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let slot_owner_id =
        insert_test_pool_oauth_account(&state, "Node Shunt Slot Owner", "oauth-soft-degrade-owner")
            .await;
    let soft_degraded_id = insert_test_pool_oauth_account(
        &state,
        "Node Shunt Soft Degraded",
        "oauth-soft-degrade-target",
    )
    .await;
    let fallback_ready_id = insert_test_pool_oauth_account(
        &state,
        "Fallback Ready Candidate",
        "oauth-soft-degrade-fallback",
    )
    .await;
    set_test_account_group_name(&state.pool, slot_owner_id, Some("soft-degrade-priority")).await;
    set_test_account_group_name(&state.pool, soft_degraded_id, Some("soft-degrade-priority")).await;
    set_test_account_group_name(
        &state.pool,
        fallback_ready_id,
        Some("soft-degrade-fallback"),
    )
    .await;

    let mut primary_rule = test_tag_routing_rule();
    primary_rule.priority_tier = TagPriorityTier::Primary;
    let primary_tag = insert_test_tag(&state.pool, "soft-degrade-owner-primary", &primary_rule)
        .await
        .expect("insert primary owner tag");
    let normal_tag = insert_test_tag(
        &state.pool,
        "soft-degrade-target-normal",
        &test_tag_routing_rule(),
    )
    .await
    .expect("insert normal target tag");
    let mut fallback_rule = test_tag_routing_rule();
    fallback_rule.priority_tier = TagPriorityTier::Fallback;
    let fallback_tag = insert_test_tag(&state.pool, "soft-degrade-ready-fallback", &fallback_rule)
        .await
        .expect("insert fallback ready tag");
    sync_account_tag_links(&state.pool, slot_owner_id, &[primary_tag.summary.id])
        .await
        .expect("attach primary owner tag");
    sync_account_tag_links(&state.pool, soft_degraded_id, &[normal_tag.summary.id])
        .await
        .expect("attach normal target tag");
    sync_account_tag_links(&state.pool, fallback_ready_id, &[fallback_tag.summary.id])
        .await
        .expect("attach fallback ready tag");

    save_soft_degrade_group_metadata(&state.pool).await;

    let resolution =
        resolve_pool_account_for_request(&state, None, &[slot_owner_id], &HashSet::new())
            .await
            .expect("resolve soft-degraded priority candidate");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected soft-degraded candidate to remain routable");
    };
    assert_eq!(account.account_id, soft_degraded_id);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::FreshAssignment
    );
    let ForwardProxyRouteScope::BoundGroup { group_name, .. } = &account.forward_proxy_scope else {
        panic!("expected soft-degraded node shunt candidate to use bound-group live fallback");
    };
    assert_eq!(group_name, "soft-degrade-priority");
}

async fn save_soft_degrade_group_metadata(pool: &SqlitePool) {
    let mut conn = pool.acquire().await.expect("acquire metadata conn");
    for (group_name, node_shunt_enabled, message) in [
        ("soft-degrade-priority", true, "save node shunt metadata"),
        ("soft-degrade-fallback", false, "save fallback metadata"),
    ] {
        save_group_metadata_record_conn(
            &mut conn,
            group_name,
            UpstreamAccountGroupMetadata {
                note: None,
                bound_proxy_keys: vec![FORWARD_PROXY_DIRECT_KEY.to_string()],
                node_shunt_enabled,
                single_account_rotation_enabled: false,
                upstream_429_retry_enabled: false,
                upstream_429_max_retries: 0,
                concurrency_limit: 0,
            },
        )
        .await
        .expect(message);
    }
}

#[test]
pub(crate) fn retry_original_node_candidates_sort_after_sendable_candidates_even_when_priority_is_higher()
 {
    let retry_original = PoolRoutingCandidateScore {
        eligibility: PoolRoutingCandidateEligibility::SoftDegraded,
        route_binding_failure_penalty: 0,
        model_route_penalty: 0,
        routing_priority_rank: 0,
        capacity_lane: PoolRoutingCandidateCapacityLane::Primary,
        dispatch_state: PoolRoutingCandidateDispatchState::RetryOriginalNode,
        single_account_rotation_enabled: false,
        secondary_reset_proximity_secs: None,
        primary_reset_proximity_secs: None,
        scarcity_score: 0.0,
        effective_load: 0,
        last_selected_at: None,
        account_id: 10,
    };
    let ready_after_migration = PoolRoutingCandidateScore {
        eligibility: PoolRoutingCandidateEligibility::SoftDegraded,
        route_binding_failure_penalty: 0,
        model_route_penalty: 0,
        routing_priority_rank: 2,
        capacity_lane: PoolRoutingCandidateCapacityLane::Primary,
        dispatch_state: PoolRoutingCandidateDispatchState::ReadyAfterMigration,
        single_account_rotation_enabled: false,
        secondary_reset_proximity_secs: None,
        primary_reset_proximity_secs: None,
        scarcity_score: 0.0,
        effective_load: 0,
        last_selected_at: None,
        account_id: 11,
    };

    assert_eq!(
        compare_pool_routing_candidate_scores(&retry_original, &ready_after_migration),
        std::cmp::Ordering::Greater
    );
    assert_eq!(
        compare_pool_routing_candidate_scores(&ready_after_migration, &retry_original),
        std::cmp::Ordering::Less
    );
}

use super::*;
