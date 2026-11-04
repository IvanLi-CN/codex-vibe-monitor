use super::*;

#[tokio::test]
async fn resolve_pool_account_for_request_applies_tighter_long_only_hard_cap() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let free_id = insert_test_pool_api_key_account(&state, "Free", "upstream-free").await;
    let team_id = insert_test_pool_api_key_account(&state, "Team", "upstream-team").await;
    let recent_seen_at = format_test_recent_active_timestamp(Utc::now());
    let now = Utc::now();

    insert_test_pool_limit_sample_with_windows(
        &state,
        free_id,
        Some("free"),
        None,
        None,
        None,
        Some(5.0),
        Some(7 * 24 * 60),
        Some(&format_utc_iso(now + ChronoDuration::days(6))),
    )
    .await;
    insert_test_pool_limit_sample_with_windows(
        &state,
        team_id,
        Some("team"),
        Some(65.0),
        Some(300),
        Some(&format_utc_iso(now + ChronoDuration::minutes(45))),
        Some(55.0),
        Some(7 * 24 * 60),
        Some(&format_utc_iso(now + ChronoDuration::days(4))),
    )
    .await;
    for sticky_key in ["sticky-free-001", "sticky-free-002"] {
        upsert_test_sticky_route_at(&state.pool, sticky_key, free_id, &recent_seen_at).await;
    }

    let account = match resolve_pool_account_for_request(state.as_ref(), None, &[], &HashSet::new())
        .await
        .expect("resolve pool account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("pool account should resolve, got {other:?}"),
    };

    assert_eq!(account.account_id, team_id);
    assert_ne!(account.account_id, free_id);
}

#[tokio::test]
async fn resolve_pool_account_for_request_counts_in_flight_reservations_toward_effective_load() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let preferred_id =
        insert_test_pool_api_key_account(&state, "Preferred", "upstream-preferred").await;
    let fallback_id =
        insert_test_pool_api_key_account(&state, "Fallback", "upstream-fallback").await;
    let recent_seen_at = format_test_recent_active_timestamp(Utc::now());
    let now = Utc::now();

    insert_test_pool_limit_sample_with_windows(
        &state,
        preferred_id,
        Some("team"),
        Some(5.0),
        Some(300),
        Some(&format_utc_iso(now + ChronoDuration::minutes(30))),
        Some(5.0),
        Some(7 * 24 * 60),
        Some(&format_utc_iso(now + ChronoDuration::days(3))),
    )
    .await;
    insert_test_pool_limit_sample_with_windows(
        &state,
        fallback_id,
        Some("team"),
        Some(25.0),
        Some(300),
        Some(&format_utc_iso(now + ChronoDuration::minutes(30))),
        Some(25.0),
        Some(7 * 24 * 60),
        Some(&format_utc_iso(now + ChronoDuration::days(3))),
    )
    .await;
    for sticky_key in ["sticky-pref-001", "sticky-pref-002"] {
        upsert_test_sticky_route_at(&state.pool, sticky_key, preferred_id, &recent_seen_at).await;
    }
    reserve_test_pool_routing_account(&state, "reservation-001", preferred_id).await;

    let account = match resolve_pool_account_for_request(state.as_ref(), None, &[], &HashSet::new())
        .await
        .expect("resolve pool account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("pool account should resolve, got {other:?}"),
    };

    assert_eq!(account.account_id, fallback_id);
    assert_ne!(account.account_id, preferred_id);
}
#[tokio::test]
async fn reserve_pool_routing_account_tracks_pinned_sticky_reuse_slots() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Sticky", "upstream-sticky").await;
    let account = PoolResolvedAccount {
        account_id,
        display_name: "sticky-account".to_string(),
        kind: "api_key_codex".to_string(),
        auth: PoolResolvedAuth::ApiKey {
            authorization: "Bearer sticky-account".to_string(),
        },
        upstream_base_url: Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        routing_source: PoolRoutingSelectionSource::StickyReuse,
        routing_control_fingerprint: None,
        sticky_affinity_generation: None,
        routing_selection_audit: None,
        priority_handoff_permit: None,
        group_name: Some(test_required_group_name().to_string()),
        bound_proxy_keys: test_required_group_bound_proxy_keys(),
        forward_proxy_scope: ForwardProxyRouteScope::PinnedProxyKey(
            FORWARD_PROXY_DIRECT_KEY.to_string(),
        ),
        single_account_rotation_enabled: false,
        upstream_429_retry_enabled: false,
        upstream_429_max_retries: 0,
        fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
        image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
        codex_imagegen_rewrite_mode: Default::default(),
        request_compression_algorithm: RequestCompressionAlgorithm::Identity,
        oauth_request_compression_algorithm: RequestCompressionAlgorithm::Zstd,
        response_endpoint_capability: CapabilitySupport::Unknown,
        chat_completions_capability: CapabilitySupport::Unknown,
        image_endpoint_capability: CapabilitySupport::Unknown,
        response_image_tool_capability: CapabilitySupport::Unknown,
        codex_imagegen_capability: CapabilitySupport::Unknown,
        standalone_search_capability: CapabilitySupport::Unknown,
    };

    reserve_pool_routing_account(state.as_ref(), "sticky-reservation", &account);

    let reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    let reservation = reservations
        .get("sticky-reservation")
        .expect("sticky reuse reservation should be recorded");
    assert_eq!(reservation.account_id, account_id);
    assert_eq!(
        reservation.proxy_key.as_deref(),
        Some(FORWARD_PROXY_DIRECT_KEY)
    );
}

#[tokio::test]
async fn resolve_pool_account_for_request_keeps_old_in_flight_reservations_counted() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let preferred_id =
        insert_test_pool_api_key_account(&state, "Preferred", "upstream-preferred").await;
    let fallback_id =
        insert_test_pool_api_key_account(&state, "Fallback", "upstream-fallback").await;
    let recent_seen_at = format_test_recent_active_timestamp(Utc::now());
    let now = Utc::now();

    insert_test_pool_limit_sample_with_windows(
        &state,
        preferred_id,
        Some("team"),
        Some(5.0),
        Some(300),
        Some(&format_utc_iso(now + ChronoDuration::minutes(30))),
        Some(5.0),
        Some(7 * 24 * 60),
        Some(&format_utc_iso(now + ChronoDuration::days(3))),
    )
    .await;
    insert_test_pool_limit_sample_with_windows(
        &state,
        fallback_id,
        Some("team"),
        Some(25.0),
        Some(300),
        Some(&format_utc_iso(now + ChronoDuration::minutes(30))),
        Some(25.0),
        Some(7 * 24 * 60),
        Some(&format_utc_iso(now + ChronoDuration::days(3))),
    )
    .await;
    for sticky_key in ["sticky-pref-001", "sticky-pref-002"] {
        upsert_test_sticky_route_at(&state.pool, sticky_key, preferred_id, &recent_seen_at).await;
    }
    state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned")
        .insert(
            "reservation-old".to_string(),
            PoolRoutingReservation {
                account_id: preferred_id,
                model: None,
                proxy_key: None,
                created_at: std::time::Instant::now() - Duration::from_secs(5 * 60),
            },
        );

    let account = match resolve_pool_account_for_request(state.as_ref(), None, &[], &HashSet::new())
        .await
        .expect("resolve pool account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("pool account should resolve, got {other:?}"),
    };

    assert_eq!(account.account_id, fallback_id);
    assert_ne!(account.account_id, preferred_id);
}

#[tokio::test]
async fn resolve_pool_account_for_request_preserves_long_only_cap_without_window_metadata() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let legacy_long_only_id =
        insert_test_pool_api_key_account(&state, "Legacy Long Only", "upstream-legacy").await;
    let team_id = insert_test_pool_api_key_account(&state, "Team", "upstream-team").await;
    let recent_seen_at = format_test_recent_active_timestamp(Utc::now());
    let now = Utc::now();

    insert_test_pool_limit_sample_with_windows(
        &state,
        legacy_long_only_id,
        Some("free"),
        None,
        None,
        None,
        Some(5.0),
        None,
        None,
    )
    .await;
    insert_test_pool_limit_sample_with_windows(
        &state,
        team_id,
        Some("team"),
        Some(25.0),
        Some(300),
        Some(&format_utc_iso(now + ChronoDuration::minutes(30))),
        Some(25.0),
        Some(7 * 24 * 60),
        Some(&format_utc_iso(now + ChronoDuration::days(3))),
    )
    .await;
    for sticky_key in ["sticky-legacy-001", "sticky-legacy-002"] {
        upsert_test_sticky_route_at(
            &state.pool,
            sticky_key,
            legacy_long_only_id,
            &recent_seen_at,
        )
        .await;
    }

    let account = match resolve_pool_account_for_request(state.as_ref(), None, &[], &HashSet::new())
        .await
        .expect("resolve pool account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("pool account should resolve, got {other:?}"),
    };

    assert_eq!(account.account_id, team_id);
    assert_ne!(account.account_id, legacy_long_only_id);
}

#[tokio::test]
async fn resolve_pool_account_for_request_preserves_local_long_limit_without_samples() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let locally_limited_id =
        insert_test_pool_api_key_account(&state, "Locally Limited", "upstream-local").await;
    let team_id = insert_test_pool_api_key_account(&state, "Team", "upstream-team").await;
    let recent_seen_at = format_test_recent_active_timestamp(Utc::now());
    let now = Utc::now();

    set_test_account_local_limits(&state.pool, locally_limited_id, None, Some(100.0)).await;
    insert_test_pool_limit_sample_with_windows(
        &state,
        team_id,
        Some("team"),
        Some(25.0),
        Some(300),
        Some(&format_utc_iso(now + ChronoDuration::minutes(30))),
        Some(25.0),
        Some(7 * 24 * 60),
        Some(&format_utc_iso(now + ChronoDuration::days(3))),
    )
    .await;
    for sticky_key in ["sticky-local-001", "sticky-local-002"] {
        upsert_test_sticky_route_at(&state.pool, sticky_key, locally_limited_id, &recent_seen_at)
            .await;
    }

    let account = match resolve_pool_account_for_request(state.as_ref(), None, &[], &HashSet::new())
        .await
        .expect("resolve pool account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("pool account should resolve, got {other:?}"),
    };

    assert_eq!(account.account_id, team_id);
    assert_ne!(account.account_id, locally_limited_id);
}

#[tokio::test]
async fn resolve_pool_account_for_request_defers_sticky_binding_until_success() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let account = match resolve_pool_account_for_request(
        state.as_ref(),
        Some("sticky-001"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve pool account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("pool account should resolve, got {other:?}"),
    };
    assert_eq!(account.account_id, account_id);
    assert!(
        load_test_sticky_route_account_id(&state.pool, "sticky-001")
            .await
            .is_none(),
        "sticky binding should not move before request success"
    );

    record_pool_route_success(
        &state.pool,
        account.account_id,
        Utc::now(),
        Some("sticky-001"),
        None,
    )
    .await
    .expect("record route success");

    assert_eq!(
        load_test_sticky_route_account_id(&state.pool, "sticky-001").await,
        Some(account_id)
    );
}
