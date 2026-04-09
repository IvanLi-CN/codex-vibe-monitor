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
        group_name: Some(test_required_group_name().to_string()),
        bound_proxy_keys: test_required_group_bound_proxy_keys(),
        forward_proxy_scope: ForwardProxyRouteScope::PinnedProxyKey(
            FORWARD_PROXY_DIRECT_KEY.to_string(),
        ),
        group_upstream_429_retry_enabled: false,
        group_upstream_429_max_retries: 0,
        fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
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

#[tokio::test]
async fn pool_route_retries_same_account_before_switching() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_retry_upstream(&[("Bearer upstream-primary", 2)]).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-001"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-primary");
    assert_eq!(payload["attempt"], 3);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);
    drop(attempts);

    let route_account_id = wait_for_test_sticky_route_account_id(&state.pool, "sticky-001")
        .await
        .expect("sticky route should be rebound after success");
    assert_eq!(route_account_id, primary_id);
    assert_ne!(route_account_id, secondary_id);

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_switches_accounts_after_same_account_retries_are_exhausted() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_retry_upstream(&[("Bearer upstream-primary", 8)]).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-002"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-secondary");
    assert_eq!(payload["attempt"], 1);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
    drop(attempts);

    let primary_status: String =
        sqlx::query_scalar("SELECT status FROM pool_upstream_accounts WHERE id = ?1")
            .bind(primary_id)
            .fetch_one(&state.pool)
            .await
            .expect("load primary status");
    assert_eq!(primary_status, "active");
    assert_eq!(
        wait_for_test_sticky_route_account_id(&state.pool, "sticky-002").await,
        Some(secondary_id)
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_switches_accounts_immediately_after_upstream_429() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_rate_limit_responses_upstream(&[("Bearer upstream-primary", 99)]).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-switch"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-secondary");
    assert_eq!(payload["attempt"], 1);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
    drop(attempts);

    wait_for_pool_upstream_request_attempts(&state.pool, 2).await;
    let attempt_rows = sqlx::query_as::<_, (i64, i64, i64, Option<String>)>(
        r#"
        SELECT attempt_index, distinct_account_index, same_account_retry_index, failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load attempt rows");
    assert_eq!(attempt_rows.len(), 2);
    assert_eq!(attempt_rows[0].0, 1);
    assert_eq!(attempt_rows[0].1, 1);
    assert_eq!(attempt_rows[0].2, 1);
    assert_eq!(
        attempt_rows[0].3.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    );
    assert_eq!(attempt_rows[1].0, 2);
    assert_eq!(attempt_rows[1].1, 2);
    assert_eq!(attempt_rows[1].2, 1);
    assert_eq!(attempt_rows[1].3, None);

    let route_account_id = wait_for_test_sticky_route_account_id(&state.pool, "sticky-429-switch")
        .await
        .expect("sticky route should move to the successful account");
    assert_eq!(route_account_id, secondary_id);
    assert_ne!(route_account_id, primary_id);

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_waits_for_recovered_alternate_after_upstream_429() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_rate_limit_responses_upstream(&[("Bearer upstream-primary", 99)]).await;
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        Duration::from_millis(180),
        Duration::from_millis(10),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    set_test_account_status(&state.pool, secondary_id, "needs_reauth").await;

    let pool = state.pool.clone();
    let release_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(40)).await;
        set_test_account_status(&pool, secondary_id, "active").await;
    });

    let started = Instant::now();
    // Under the full 900+ test suite, Tokio wakeups can drift far past the nominal
    // sub-200ms recovery window. Guard against hangs without pinning the assertion to
    // a scheduler-sensitive wall-clock upper bound.
    let response = tokio::time::timeout(
        Duration::from_secs(3),
        proxy_openai_v1(
            State(state.clone()),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::from_iter([(
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            )]),
            Body::from(
                r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-wait-recovered"}"#
                    .as_bytes()
                    .to_vec(),
            ),
        ),
    )
    .await
    .expect("bounded wait request should not hang");
    let elapsed = started.elapsed();

    release_task
        .await
        .expect("alternate release task should join");

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        elapsed >= Duration::from_millis(35),
        "request should wait for the alternate to recover, elapsed={elapsed:?}"
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-secondary");
    assert_eq!(payload["attempt"], 1);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_group_without_upstream_429_retry_switches_accounts_immediately() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_rate_limit_responses_upstream(&[("Bearer upstream-primary", 99)]).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Primary",
        "upstream-primary",
        Some("latam"),
        None,
        None,
    )
    .await;
    insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-group-off"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-secondary");
    assert_eq!(payload["attempt"], 1);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_group_upstream_429_retry_retries_same_account_before_succeeding() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_rate_limit_responses_upstream(&[("Bearer upstream-primary", 2)]).await;
    let base_state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    let state =
        clone_state_with_pool_group_429_retry_delay_override(&base_state, Some(Duration::ZERO));
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Primary",
        "upstream-primary",
        Some("latam"),
        None,
        None,
    )
    .await;
    insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

    let retry_payload: UpdateUpstreamAccountGroupRequest = serde_json::from_value(json!({
        "upstream429RetryEnabled": true,
        "upstream429MaxRetries": 2
    }))
    .expect("deserialize retry payload");
    let _ = update_upstream_account_group(
        State(state.clone()),
        HeaderMap::new(),
        axum::extract::Path("latam".to_string()),
        Json(retry_payload),
    )
    .await
    .expect("enable group 429 retry");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-group-retry"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-primary");
    assert_eq!(payload["attempt"], 3);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);
    drop(attempts);

    wait_for_pool_upstream_request_attempts(&state.pool, 3).await;
    let attempt_rows = sqlx::query_as::<_, (i64, i64, i64, Option<String>)>(
        r#"
        SELECT attempt_index, distinct_account_index, same_account_retry_index, failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load retry attempt rows");
    assert_eq!(attempt_rows.len(), 3);
    assert_eq!(
        attempt_rows[0],
        (
            1,
            1,
            1,
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429.to_string())
        )
    );
    assert_eq!(
        attempt_rows[1],
        (
            2,
            1,
            2,
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429.to_string())
        )
    );
    assert_eq!(attempt_rows[2].0, 3);
    assert_eq!(attempt_rows[2].1, 1);
    assert_eq!(attempt_rows[2].2, 3);
    assert_eq!(attempt_rows[2].3, None);

    let route_account_id =
        wait_for_test_sticky_route_account_id(&state.pool, "sticky-429-group-retry")
            .await
            .expect("sticky route should stay on primary account");
    assert_eq!(route_account_id, primary_id);

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_group_upstream_429_retry_keeps_separate_budget_from_server_errors() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_sequential_failure_responses_upstream(vec![(
            "Bearer upstream-primary",
            vec![
                StatusCode::INTERNAL_SERVER_ERROR,
                StatusCode::TOO_MANY_REQUESTS,
                StatusCode::TOO_MANY_REQUESTS,
            ],
        )])
        .await;
    let base_state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    let state =
        clone_state_with_pool_group_429_retry_delay_override(&base_state, Some(Duration::ZERO));
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Primary",
        "upstream-primary",
        Some("latam"),
        None,
        None,
    )
    .await;
    insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

    let retry_payload: UpdateUpstreamAccountGroupRequest = serde_json::from_value(json!({
        "upstream429RetryEnabled": true,
        "upstream429MaxRetries": 2
    }))
    .expect("deserialize retry payload");
    let _ = update_upstream_account_group(
        State(state.clone()),
        HeaderMap::new(),
        axum::extract::Path("latam".to_string()),
        Json(retry_payload),
    )
    .await
    .expect("enable group 429 retry");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-mixed-budget"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-primary");
    assert_eq!(payload["attempt"], 4);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(4));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);
    drop(attempts);

    wait_for_pool_upstream_request_attempts(&state.pool, 4).await;
    let attempt_rows = sqlx::query_as::<_, (i64, i64, i64, Option<String>)>(
        r#"
        SELECT attempt_index, distinct_account_index, same_account_retry_index, failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load retry attempt rows");
    assert_eq!(attempt_rows.len(), 4);
    assert_eq!(
        attempt_rows[0],
        (
            1,
            1,
            1,
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX.to_string())
        )
    );
    assert_eq!(
        attempt_rows[1],
        (
            2,
            1,
            2,
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429.to_string())
        )
    );
    assert_eq!(
        attempt_rows[2],
        (
            3,
            1,
            3,
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429.to_string())
        )
    );
    assert_eq!(attempt_rows[3].0, 4);
    assert_eq!(attempt_rows[3].1, 1);
    assert_eq!(attempt_rows[3].2, 4);
    assert_eq!(attempt_rows[3].3, None);

    let route_account_id =
        wait_for_test_sticky_route_account_id(&state.pool, "sticky-429-mixed-budget")
            .await
            .expect("sticky route should stay on primary account");
    assert_eq!(route_account_id, primary_id);

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_switches_accounts_immediately_after_upstream_402() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_static_failure_responses_upstream(
        &[("Bearer upstream-primary", StatusCode::PAYMENT_REQUIRED)],
    )
    .await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-402-switch"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-secondary");

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
    drop(attempts);

    let primary_status: String =
        sqlx::query_scalar("SELECT status FROM pool_upstream_accounts WHERE id = ?1")
            .bind(primary_id)
            .fetch_one(&state.pool)
            .await
            .expect("load primary status");
    assert_eq!(primary_status, "error");
    assert_eq!(
        wait_for_test_sticky_route_account_id(&state.pool, "sticky-402-switch").await,
        Some(secondary_id)
    );

    wait_for_pool_upstream_request_attempts(&state.pool, 2).await;
    let attempt_rows = sqlx::query_as::<_, (i64, Option<String>)>(
        r#"
        SELECT distinct_account_index, failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load attempt rows");
    assert_eq!(attempt_rows[0].0, 1);
    assert_eq!(
        attempt_rows[0].1.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_402)
    );
    assert_eq!(attempt_rows[1].0, 2);
    assert_eq!(attempt_rows[1].1, None);

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_live_request_switches_accounts_immediately_after_upstream_429() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_rate_limit_echo_upstream(&[("Bearer upstream-primary", 99)]).await;
    let state = test_state_with_openai_base_body_limit_and_read_timeout(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
        Duration::from_millis(50),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/echo?mode=pool-live-429".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from("hello-live-429"),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode upstream payload");
    assert_eq!(payload["authorization"], "Bearer upstream-secondary");
    assert_eq!(payload["attempt"], 1);
    assert_eq!(payload["body"], "hello-live-429");

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_retries_first_chunk_failure_before_switching() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_first_chunk_retry_upstream(&[("Bearer upstream-primary", 2)]).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-003"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-primary");
    assert_eq!(payload["attempt"], 3);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);
    drop(attempts);

    let route_account_id = wait_for_test_sticky_route_account_id(&state.pool, "sticky-003")
        .await
        .expect("sticky route should remain on the recovered account");
    assert_eq!(route_account_id, primary_id);
    assert_ne!(route_account_id, secondary_id);

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_non_responses_timeouts_retry_same_account_before_switching() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        upstream_account_id: Option<i64>,
        distinct_account_index: i64,
        same_account_retry_index: i64,
        status: String,
        failure_kind: Option<String>,
    }

    let (upstream_base, attempts, upstream_handle) = spawn_pool_delayed_headers_retry_upstream(
        Duration::from_millis(400),
        &[("Bearer upstream-primary", 2)],
    )
    .await;
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.request_timeout = Duration::from_millis(150);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Primary",
        "upstream-primary",
        None,
        None,
        Some(upstream_base.as_str()),
    )
    .await;
    let secondary_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Secondary",
        "upstream-secondary",
        None,
        None,
        Some(upstream_base.as_str()),
    )
    .await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/chat/completions".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            ),
            (
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ]),
        Body::from(
            r#"{"model":"gpt-5","messages":[{"role":"user","content":"hello"}],"stickyKey":"sticky-non-responses-timeout-retry-001"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read non-responses retry response");
    let payload: Value = serde_json::from_slice(&body).expect("decode non-responses retry body");

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 3).await;
    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT
            upstream_account_id,
            distinct_account_index,
            same_account_retry_index,
            status,
            failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load non-responses timeout retry rows");
    assert!(
        attempt_rows.len() == 3 || attempt_rows.len() == 4,
        "expected 3 or 4 attempts, got {}",
        attempt_rows.len()
    );
    let is_timeout_or_stream_failure = |row: &AttemptRouteRow| {
        row.status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
            && row.failure_kind.as_deref().is_some_and(|kind| {
                kind == PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT
                    || kind == PROXY_FAILURE_UPSTREAM_STREAM_ERROR
            })
    };
    assert_eq!(attempt_rows[0].upstream_account_id, Some(primary_id));
    assert_eq!(attempt_rows[0].distinct_account_index, 1);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert!(is_timeout_or_stream_failure(&attempt_rows[0]));
    assert_eq!(attempt_rows[1].upstream_account_id, Some(primary_id));
    assert_eq!(attempt_rows[1].distinct_account_index, 1);
    assert_eq!(attempt_rows[1].same_account_retry_index, 2);
    assert!(is_timeout_or_stream_failure(&attempt_rows[1]));
    assert_eq!(attempt_rows[2].upstream_account_id, Some(primary_id));
    assert_eq!(attempt_rows[2].distinct_account_index, 1);
    assert_eq!(attempt_rows[2].same_account_retry_index, 3);
    let sticky_account_id = wait_for_test_sticky_route_account_id(
        &state.pool,
        "sticky-non-responses-timeout-retry-001",
    )
    .await
    .expect("sticky route should be recorded after success");

    let attempts = attempts
        .lock()
        .expect("lock delayed headers retry attempts");
    match payload["authorization"].as_str() {
        Some("Bearer upstream-primary") => {
            assert_eq!(payload["attempt"], 3);
            assert_eq!(attempt_rows.len(), 3);
            assert_eq!(
                attempt_rows[2].status,
                POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
            );
            assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
            assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);
            assert_eq!(sticky_account_id, primary_id);
        }
        Some("Bearer upstream-secondary") => {
            assert_eq!(payload["attempt"], 1);
            assert_eq!(attempt_rows.len(), 4);
            assert!(is_timeout_or_stream_failure(&attempt_rows[2]));
            assert_eq!(attempt_rows[3].upstream_account_id, Some(secondary_id));
            assert_eq!(attempt_rows[3].distinct_account_index, 2);
            assert_eq!(attempt_rows[3].same_account_retry_index, 1);
            assert_eq!(
                attempt_rows[3].status,
                POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
            );
            assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
            assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
            assert_eq!(sticky_account_id, secondary_id);
        }
        other => panic!("unexpected upstream authorization: {other:?}"),
    }
    drop(attempts);

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_switches_accounts_after_first_chunk_failures_are_exhausted() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_first_chunk_retry_upstream(&[("Bearer upstream-primary", 8)]).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-004"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-secondary");
    assert_eq!(payload["attempt"], 1);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
    drop(attempts);

    let primary_status: String =
        sqlx::query_scalar("SELECT status FROM pool_upstream_accounts WHERE id = ?1")
            .bind(primary_id)
            .fetch_one(&state.pool)
            .await
            .expect("load primary status");
    assert_eq!(primary_status, "active");
    assert_eq!(
        wait_for_test_sticky_route_account_id(&state.pool, "sticky-004").await,
        Some(secondary_id)
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn capture_target_pool_route_retries_first_chunk_failure_and_persists_single_invocation() {
    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        status: Option<String>,
        payload: Option<String>,
    }

    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_first_chunk_retry_upstream(&[("Bearer upstream-primary", 2)]).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

    let request_payload = json!({
        "model": "gpt-5.2-codex",
        "stream": false,
        "input": "hello",
        "stickyKey": "sticky-cap-001",
    });
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            serde_json::to_vec(&request_payload).expect("serialize capture retry request body"),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode capture retry payload");
    assert_eq!(payload["authorization"], "Bearer upstream-primary");
    assert_eq!(payload["attempt"], 3);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);
    drop(attempts);

    wait_for_codex_invocations(&state.pool, 1).await;
    assert_eq!(count_codex_invocations(&state.pool).await, 1);
    assert_eq!(count_request_forward_proxy_attempts(&state.pool).await, 0);

    let row = sqlx::query_as::<_, PersistedRow>(
        r#"
        SELECT status, payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_optional(&state.pool)
    .await
    .expect("query capture record")
    .expect("capture record should be persisted");
    assert_eq!(row.status.as_deref(), Some("success"));

    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("capture payload should be present"),
    )
    .expect("decode capture payload");
    assert_eq!(payload_json["upstreamScope"].as_str(), Some("internal"));
    assert_eq!(payload_json["routeMode"].as_str(), Some("pool"));
    assert_eq!(payload_json["stickyKey"].as_str(), Some("sticky-cap-001"));
    assert_eq!(payload_json["upstreamAccountId"].as_i64(), Some(primary_id));
    assert_eq!(
        payload_json["upstreamAccountName"].as_str(),
        Some("Primary")
    );
    assert!(payload_json["proxyDisplayName"].is_null());
    assert_eq!(
        wait_for_test_sticky_route_account_id(&state.pool, "sticky-cap-001").await,
        Some(primary_id)
    );
    assert_ne!(primary_id, secondary_id);

    upstream_handle.abort();
}

#[tokio::test]
async fn capture_target_pool_route_no_content_success_finalizes_pending_attempt() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let request_payload = json!({
        "model": "gpt-5.4",
        "stream": false,
        "input": "hello",
        "stickyKey": "sticky-cap-204",
    });
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses?mode=no-content".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            serde_json::to_vec(&request_payload)
                .expect("serialize capture no-content request body"),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read no-content response body");
    assert!(body.is_empty(), "204 response body should stay empty");

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;

    let mut attempt_row = None;
    for _ in 0..20 {
        attempt_row = sqlx::query_as::<
            _,
            (
                String,
                Option<String>,
                Option<i64>,
                Option<f64>,
                Option<String>,
                Option<i64>,
            ),
        >(
            r#"
            SELECT
                status,
                finished_at,
                http_status,
                stream_latency_ms,
                failure_kind,
                upstream_account_id
            FROM pool_upstream_request_attempts
            WHERE invoke_id LIKE 'proxy-%'
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query pool attempt row");
        if attempt_row
            .as_ref()
            .is_some_and(|row| row.0 == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let attempt_row = attempt_row.expect("pool attempt row should be persisted");

    assert_eq!(attempt_row.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS);
    assert!(
        attempt_row
            .1
            .as_deref()
            .is_some_and(|value| !value.is_empty())
    );
    assert_eq!(attempt_row.2, Some(204));
    assert_eq!(attempt_row.3, Some(0.0));
    assert_eq!(attempt_row.4, None);
    assert_eq!(attempt_row.5, Some(primary_id));

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_surfaces_last_upstream_error_when_failover_is_exhausted() {
    let (upstream_base, _attempts, upstream_handle) =
        spawn_pool_retry_upstream(&[("Bearer upstream-primary", 99)]).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-500"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert_eq!(
        payload["error"].as_str(),
        Some("pool upstream responded with 500")
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_returns_clear_429_when_only_account_is_rate_limited() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_rate_limit_responses_upstream(&[("Bearer upstream-primary", 99)]).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-terminal"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert_eq!(
        payload["error"].as_str(),
        Some(POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE)
    );

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_returns_clear_503_when_all_accounts_are_temporarily_degraded() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    set_test_account_degraded_route_state(
        &state.pool,
        primary_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        "test degraded plain 429",
    )
    .await;
    set_test_account_degraded_route_state(
        &state.pool,
        secondary_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX,
        "test degraded 5xx",
    )
    .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-degraded-only"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert_eq!(
        payload["error"].as_str(),
        Some(POOL_ALL_ACCOUNTS_DEGRADED_MESSAGE)
    );
}

#[tokio::test]
async fn pool_route_returns_clear_429_when_all_accounts_are_already_in_429_cooldown() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    set_test_account_rate_limited_cooldown(&state.pool, primary_id, 120).await;
    set_test_account_rate_limited_cooldown(&state.pool, secondary_id, 120).await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-cooldown"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert_eq!(
        payload["error"].as_str(),
        Some(POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE)
    );
}

#[tokio::test]
async fn pool_route_ignores_missing_credentials_when_all_routable_accounts_are_rate_limited() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let missing_credentials_id =
        insert_test_pool_api_key_account(&state, "Missing Credentials", "upstream-missing").await;
    set_test_account_rate_limited_cooldown(&state.pool, primary_id, 120).await;
    clear_test_account_credentials(&state.pool, missing_credentials_id).await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-missing-creds"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert_eq!(
        payload["error"].as_str(),
        Some(POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE)
    );
}

#[tokio::test]
async fn pool_route_stale_sticky_binding_does_not_hide_pool_wide_429() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    set_test_account_status(&state.pool, primary_id, "needs_reauth").await;
    set_test_account_rate_limited_cooldown(&state.pool, secondary_id, 120).await;
    let sticky_seen_at = format_utc_iso(Utc::now());
    upsert_test_sticky_route_at(
        &state.pool,
        "sticky-429-stale-binding",
        primary_id,
        &sticky_seen_at,
    )
    .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-stale-binding"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert_eq!(
        payload["error"].as_str(),
        Some(POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE)
    );
}

#[tokio::test]
async fn pool_route_missing_credentials_sticky_binding_does_not_hide_pool_wide_429() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    clear_test_account_credentials(&state.pool, primary_id).await;
    set_test_account_rate_limited_cooldown(&state.pool, secondary_id, 120).await;
    let sticky_seen_at = format_utc_iso(Utc::now());
    upsert_test_sticky_route_at(
        &state.pool,
        "sticky-429-missing-creds-binding",
        primary_id,
        &sticky_seen_at,
    )
    .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-missing-creds-binding"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert_eq!(
        payload["error"].as_str(),
        Some(POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE)
    );
}

#[tokio::test]
async fn pool_route_keeps_generic_no_candidate_when_other_accounts_are_unavailable_for_other_reasons()
 {
    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_rate_limit_responses_upstream(&[("Bearer upstream-primary", 99)]).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    set_test_account_status(&state.pool, secondary_id, "needs_reauth").await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-mixed-no-candidate"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response
            .headers()
            .get(http_header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok()),
        Some("10")
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert_eq!(
        payload["error"].as_str(),
        Some(POOL_NO_AVAILABLE_ACCOUNT_MESSAGE)
    );

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_waits_for_header_sticky_account_before_first_attempt() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        Duration::from_millis(180),
        Duration::from_millis(10),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let delayed_id = insert_test_pool_api_key_account(&state, "Delayed", "upstream-delayed").await;
    set_test_account_status(&state.pool, delayed_id, "needs_reauth").await;

    let request_state = state.clone();
    let request_task = tokio::spawn(async move {
        proxy_openai_v1(
            State(request_state),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::from_iter([
                (
                    http_header::AUTHORIZATION,
                    HeaderValue::from_static("Bearer pool-live-key"),
                ),
                (
                    HeaderName::from_static("x-sticky-key"),
                    HeaderValue::from_static("sticky-wait-header"),
                ),
            ]),
            Body::from(r#"{"model":"gpt-5","input":"hello"}"#.as_bytes().to_vec()),
        )
        .await
    });

    let pool = state.pool.clone();
    let release_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(40)).await;
        set_test_account_status(&pool, delayed_id, "active").await;
    });

    let response = request_task
        .await
        .expect("header sticky request task should join");
    release_task
        .await
        .expect("delayed account release task should join");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-delayed");
    assert_eq!(payload["attempt"], 1);
    wait_for_pool_attempt_row_count(&state.pool, 1).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-delayed").copied(), Some(1));

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_waits_for_recovered_alternate_after_upstream_failure() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_static_failure_responses_upstream(
        &[("Bearer upstream-primary", StatusCode::INTERNAL_SERVER_ERROR)],
    )
    .await;
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        Duration::from_millis(180),
        Duration::from_millis(10),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let delayed_id = insert_test_pool_api_key_account(&state, "Delayed", "upstream-delayed").await;
    set_test_account_status(&state.pool, delayed_id, "needs_reauth").await;

    let request_state = state.clone();
    let request_task = tokio::spawn(async move {
        proxy_openai_v1(
            State(request_state),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::from_iter([(
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            )]),
            Body::from(
                r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-recover-after-upstream-failure"}"#
                    .as_bytes()
                    .to_vec(),
            ),
        )
        .await
    });

    let pool = state.pool.clone();
    let release_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(40)).await;
        set_test_account_status(&pool, delayed_id, "active").await;
    });

    let response = request_task.await.expect("request task should join");
    release_task
        .await
        .expect("delayed account release task should join");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-delayed");
    assert_eq!(payload["attempt"], 1);

    wait_for_pool_attempt_row_count(&state.pool, 4).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 4);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-delayed").copied(), Some(1));

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_existing_sticky_owner_waits_for_recovered_alternate_after_upstream_failure() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_static_failure_responses_upstream(
        &[("Bearer upstream-primary", StatusCode::INTERNAL_SERVER_ERROR)],
    )
    .await;
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        Duration::from_millis(180),
        Duration::from_millis(10),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let delayed_id = insert_test_pool_api_key_account(&state, "Delayed", "upstream-delayed").await;
    upsert_test_sticky_route_at(
        &state.pool,
        "sticky-existing-owner-wait-recovered",
        primary_id,
        &format_utc_iso(Utc::now()),
    )
    .await;
    set_test_account_generic_route_cooldown(&state.pool, primary_id, 120).await;
    set_test_account_status(&state.pool, delayed_id, "needs_reauth").await;

    let request_state = state.clone();
    let request_task = tokio::spawn(async move {
        proxy_openai_v1(
            State(request_state),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::from_iter([(
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            )]),
            Body::from(
                r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-existing-owner-wait-recovered"}"#
                    .as_bytes()
                    .to_vec(),
            ),
        )
        .await
    });

    let pool = state.pool.clone();
    let release_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(40)).await;
        set_test_account_status(&pool, delayed_id, "active").await;
    });

    let response = request_task.await.expect("request task should join");
    release_task
        .await
        .expect("delayed account release task should join");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-delayed");
    assert_eq!(payload["attempt"], 1);

    wait_for_pool_attempt_row_count(&state.pool, 4).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 4);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-delayed").copied(), Some(1));
    drop(attempts);

    let mut route_account_id =
        load_test_sticky_route_account_id(&state.pool, "sticky-existing-owner-wait-recovered")
            .await;
    for _ in 0..20 {
        if route_account_id == Some(delayed_id) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
        route_account_id =
            load_test_sticky_route_account_id(&state.pool, "sticky-existing-owner-wait-recovered")
                .await;
    }
    assert_eq!(route_account_id, Some(delayed_id));

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_body_sticky_returns_503_after_wait_timeout() {
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        Duration::from_millis(60),
        Duration::from_millis(10),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let blocked_id = insert_test_pool_api_key_account(&state, "Blocked", "upstream-blocked").await;
    set_test_account_status(&state.pool, blocked_id, "needs_reauth").await;

    let started = Instant::now();
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-wait-body-timeout"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert!(
        started.elapsed() >= Duration::from_millis(50),
        "request should wait roughly the bounded window before failing"
    );
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response
            .headers()
            .get(http_header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok()),
        Some("10")
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert_eq!(
        payload["error"].as_str(),
        Some(POOL_NO_AVAILABLE_ACCOUNT_MESSAGE)
    );
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);
}

#[tokio::test]
async fn pool_route_body_sticky_wait_timeout_returns_total_timeout_error_before_first_attempt() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.pool_upstream_responses_total_timeout = Duration::from_millis(45);
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(220),
            poll_interval: Duration::from_millis(10),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let blocked_id = insert_test_pool_api_key_account(&state, "Blocked", "upstream-blocked").await;
    set_test_account_status(&state.pool, blocked_id, "needs_reauth").await;

    let started = Instant::now();
    // This path only needs to prove the request terminates as a pre-attempt total-timeout
    // failure without hanging; exact wall-clock wakeups are too noisy under full-suite load.
    let response = tokio::time::timeout(
        Duration::from_secs(3),
        proxy_openai_v1(
            State(state.clone()),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::from_iter([(
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            )]),
            Body::from(
                r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-wait-total-timeout"}"#
                    .as_bytes()
                    .to_vec(),
            ),
        ),
    )
    .await
    .expect("pre-attempt total-timeout request should not hang");
    let elapsed = started.elapsed();

    assert!(
        elapsed >= Duration::from_millis(25),
        "request should still wait briefly before failing, elapsed={elapsed:?}"
    );
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(
        response
            .headers()
            .get(http_header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok()),
        None
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert_eq!(
        payload["error"].as_str(),
        Some(pool_total_timeout_exhausted_message(Duration::from_millis(45)).as_str())
    );
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);
}

#[tokio::test]
async fn resolve_pool_account_for_request_with_wait_respects_external_deadline() {
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        Duration::from_secs(2),
        Duration::from_secs(1),
    )
    .await;
    let blocked_id = insert_test_pool_api_key_account(&state, "Blocked", "upstream-blocked").await;
    set_test_account_status(&state.pool, blocked_id, "needs_reauth").await;

    let started = Instant::now();
    let mut wait_deadline = None;
    let resolution = resolve_pool_account_for_request_with_wait(
        state.as_ref(),
        None,
        &[],
        &HashSet::new(),
        None,
        true,
        &mut wait_deadline,
        Some(Instant::now() + Duration::from_millis(40)),
    )
    .await
    .expect("helper resolution should succeed");
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_millis(700),
        "helper should stop on the external deadline instead of sleeping for the full wait window, elapsed={elapsed:?}"
    );
    assert!(
        matches!(
            resolution,
            PoolAccountResolutionWithWait::TotalTimeoutExpired
        ),
        "expected helper to stop on the external deadline, got {resolution:?}"
    );
    assert!(
        wait_deadline.is_some(),
        "bounded waits should record the deadline once they actually start"
    );
}

#[tokio::test]
async fn resolve_pool_account_for_request_with_wait_rejects_recovery_after_external_deadline() {
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        Duration::from_secs(2),
        Duration::from_millis(10),
    )
    .await;
    let blocked_id = insert_test_pool_api_key_account(&state, "Blocked", "upstream-blocked").await;
    let delayed_id = insert_test_pool_api_key_account(&state, "Delayed", "upstream-delayed").await;
    set_test_account_status(&state.pool, blocked_id, "needs_reauth").await;
    set_test_account_status(&state.pool, delayed_id, "needs_reauth").await;

    let pool = state.pool.clone();
    let delayed_release_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(70)).await;
        set_test_account_status(&pool, delayed_id, "active").await;
    });

    let started = Instant::now();
    let mut wait_deadline = None;
    let resolution = resolve_pool_account_for_request_with_wait(
        state.as_ref(),
        None,
        &[],
        &HashSet::new(),
        None,
        true,
        &mut wait_deadline,
        Some(Instant::now() + Duration::from_millis(40)),
    )
    .await
    .expect("helper resolution should succeed");
    let elapsed = started.elapsed();

    delayed_release_task
        .await
        .expect("delayed release task should join");

    assert!(
        elapsed < Duration::from_millis(200),
        "helper should stop on the external deadline before late recovery, elapsed={elapsed:?}"
    );
    assert!(
        matches!(
            resolution,
            PoolAccountResolutionWithWait::TotalTimeoutExpired
        ),
        "late recovery after the deadline must not be accepted, got {resolution:?}"
    );
}

#[tokio::test]
async fn pool_route_wait_timeout_overrides_stale_upstream_failure_with_503() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_static_failure_responses_upstream(
        &[("Bearer upstream-primary", StatusCode::INTERNAL_SERVER_ERROR)],
    )
    .await;
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        Duration::from_millis(60),
        Duration::from_millis(10),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let blocked_id = insert_test_pool_api_key_account(&state, "Blocked", "upstream-blocked").await;
    set_test_account_status(&state.pool, blocked_id, "needs_reauth").await;

    let started = Instant::now();
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-wait-stale-upstream-timeout"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert!(
        started.elapsed() >= Duration::from_millis(50),
        "request should wait roughly the bounded window before failing"
    );
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response
            .headers()
            .get(http_header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok()),
        Some("10")
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert_eq!(
        payload["error"].as_str(),
        Some(POOL_NO_AVAILABLE_ACCOUNT_MESSAGE)
    );

    wait_for_pool_attempt_row_count(&state.pool, 3).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 3);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-blocked").copied(), None);

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_existing_sticky_owner_retries_before_cutting_out_to_healthy_alternate() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_static_failure_responses_upstream(
        &[("Bearer upstream-primary", StatusCode::INTERNAL_SERVER_ERROR)],
    )
    .await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    upsert_test_sticky_route_at(
        &state.pool,
        "sticky-existing-owner-cutout",
        primary_id,
        &format_utc_iso(Utc::now()),
    )
    .await;
    set_test_account_generic_route_cooldown(&state.pool, primary_id, 120).await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-existing-owner-cutout"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-secondary");
    assert_eq!(payload["attempt"], 1);

    wait_for_pool_attempt_row_count(&state.pool, 4).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 4);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
    drop(attempts);

    let mut route_account_id =
        load_test_sticky_route_account_id(&state.pool, "sticky-existing-owner-cutout").await;
    for _ in 0..20 {
        if route_account_id == Some(secondary_id) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
        route_account_id =
            load_test_sticky_route_account_id(&state.pool, "sticky-existing-owner-cutout").await;
    }
    assert_eq!(
        route_account_id,
        Some(secondary_id),
        "sticky binding should move only after the alternate succeeds",
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_existing_sticky_owner_preserves_last_failure_when_cutout_target_is_unusable() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptStatusRow {
        status: String,
        failure_kind: Option<String>,
    }

    let (upstream_base, attempts, upstream_handle) = spawn_pool_static_failure_responses_upstream(
        &[("Bearer upstream-primary", StatusCode::INTERNAL_SERVER_ERROR)],
    )
    .await;
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        Duration::from_millis(180),
        Duration::from_millis(10),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let unusable_id =
        insert_test_pool_api_key_account(&state, "Unusable", "upstream-secondary").await;
    clear_test_account_credentials(&state.pool, unusable_id).await;
    upsert_test_sticky_route_at(
        &state.pool,
        "sticky-existing-owner-preserve-last-error",
        primary_id,
        &format_utc_iso(Utc::now()),
    )
    .await;
    set_test_account_generic_route_cooldown(&state.pool, primary_id, 120).await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-existing-owner-preserve-last-error"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        response.headers().get(http_header::RETRY_AFTER).is_none(),
        "sticky owner fallback should preserve the upstream failure instead of advertising pool wait"
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|message| message.contains("pool upstream responded with 500")),
        "unexpected preserved upstream failure payload: {payload}"
    );

    wait_for_pool_attempt_row_count(&state.pool, 4).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 4);

    let attempt_rows = sqlx::query_as::<_, AttemptStatusRow>(
        r#"
        SELECT status, failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load preserved sticky-owner attempt rows");
    assert_eq!(attempt_rows.len(), 4);
    assert_eq!(
        attempt_rows[3].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL
    );
    assert_eq!(
        attempt_rows[3].failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX)
    );

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);
    drop(attempts);

    assert_eq!(
        load_test_sticky_route_account_id(
            &state.pool,
            "sticky-existing-owner-preserve-last-error",
        )
        .await,
        Some(primary_id),
        "sticky binding should stay on the original owner when cut-out never succeeds",
    );

    upstream_handle.abort();
}
