use super::*;
use serde_json::json;

fn run_routing_failover_future_with_large_stack<T, Fut>(future: Fut) -> T
where
    T: Send + 'static,
    Fut: std::future::Future<Output = T> + Send + 'static,
{
    std::thread::Builder::new()
        .name("routing-failover-large-stack".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build large-stack test runtime")
                .block_on(future)
        })
        .expect("spawn large-stack test worker")
        .join()
        .expect("join large-stack test worker")
}

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

    {
        let attempts = attempts.lock().expect("lock attempts");
        assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
        assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);
    }

    let route_account_id = wait_for_test_sticky_route_account_id(&state.pool, "sticky-001")
        .await
        .expect("sticky route should be rebound after success");
    assert_eq!(route_account_id, primary_id);
    assert_ne!(route_account_id, secondary_id);

    upstream_handle.abort();
}

#[test]
fn pool_route_switches_accounts_after_same_account_retries_are_exhausted() {
    run_routing_failover_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) =
            spawn_pool_retry_upstream(&[("Bearer upstream-primary", 8)]).await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let primary_id =
            insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
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

        {
            let attempts = attempts.lock().expect("lock attempts");
            assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
            assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
        }

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
    });
}

#[test]
fn pool_route_switches_accounts_immediately_after_upstream_429() {
    run_routing_failover_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) =
            spawn_pool_rate_limit_responses_upstream(&[("Bearer upstream-primary", 99)]).await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let primary_id =
            insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
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

        {
            let attempts = attempts.lock().expect("lock attempts");
            assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
            assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
        }

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

        let route_account_id =
            wait_for_test_sticky_route_account_id(&state.pool, "sticky-429-switch")
                .await
                .expect("sticky route should move to the successful account");
        assert_eq!(route_account_id, secondary_id);
        assert_ne!(route_account_id, primary_id);

        upstream_handle.abort();
    });
}

#[test]
fn pool_route_waits_for_recovered_alternate_after_upstream_429() {
    run_routing_failover_future_with_large_stack(async move {
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

        let wait_started_rx = crate::proxy::register_pool_no_available_wait_hook(&state);
        let pool = state.pool.clone();
        let runtime_handle = tokio::runtime::Handle::current();
        let release_task = std::thread::spawn(move || {
            wait_started_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("request should signal once the bounded wait starts");
            std::thread::sleep(Duration::from_millis(40));
            runtime_handle.block_on(async move {
                set_test_account_status(&pool, secondary_id, "active").await;
            });
        });

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
                r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-wait-recovered"}"#
                    .as_bytes()
                    .to_vec(),
            ),
        )
        .await;
        let elapsed = started.elapsed();

        release_task
            .join()
            .expect("alternate release thread should join");

        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            elapsed >= Duration::from_millis(35),
            "request should wait for the alternate to recover, elapsed={elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(900),
            "bounded wait should still stay within a loaded-runner recovery budget, elapsed={elapsed:?}"
        );
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read proxy response");
        let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
        assert_eq!(payload["authorization"], "Bearer upstream-secondary");
        assert_eq!(payload["attempt"], 1);

        {
            let attempts = attempts.lock().expect("lock attempts");
            assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
            assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
        }

        upstream_handle.abort();
    });
}

#[test]
fn pool_route_group_without_upstream_429_retry_switches_accounts_immediately() {
    run_routing_failover_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) =
            spawn_pool_rate_limit_responses_upstream(&[("Bearer upstream-primary", 99)]).await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
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

        drop(attempts);
        upstream_handle.abort();
    });
}

#[test]
fn pool_route_transit_upstream_429_switches_account_without_group_retry() {
    run_routing_failover_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) =
            spawn_pool_rate_limit_responses_upstream(&[("Bearer upstream-primary", 2)]).await;
        let base_state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
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
        assert_eq!(payload["authorization"], "Bearer upstream-secondary");
        assert_eq!(payload["attempt"], 1);

        {
            let attempts = attempts.lock().expect("lock attempts");
            assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
            assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
        }

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
        .expect("load retry attempt rows");
        assert_eq!(attempt_rows.len(), 2);
        assert_eq!(
            attempt_rows[0],
            (
                1,
                1,
                1,
                Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429.to_string())
            )
        );
        assert_eq!(attempt_rows[1], (2, 2, 1, None));

        let route_account_id =
            wait_for_test_sticky_route_account_id(&state.pool, "sticky-429-group-retry")
                .await
                .expect("sticky route should stay on primary account");
        assert_ne!(route_account_id, primary_id);

        upstream_handle.abort();
    });
}

#[test]
fn pool_route_transit_upstream_429_uses_account_failover_after_server_error() {
    run_routing_failover_future_with_large_stack(async move {
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
        let base_state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
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

        assert_transit_429_failover_result(&state, &attempts, response, primary_id).await;

        upstream_handle.abort();
    });
}

async fn assert_transit_429_failover_result(
    state: &Arc<AppState>,
    attempts: &Arc<StdMutex<HashMap<String, usize>>>,
    response: axum::response::Response,
    primary_id: i64,
) {
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-secondary");
    assert_eq!(payload["attempt"], 1);
    {
        let attempts = attempts.lock().expect("lock attempts");
        assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(2));
        assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
    }
    wait_for_pool_upstream_request_attempts(&state.pool, 3).await;
    let attempt_rows = sqlx::query_as::<_, (i64, i64, i64, Option<String>)>(
        "SELECT attempt_index, distinct_account_index, same_account_retry_index, failure_kind FROM pool_upstream_request_attempts ORDER BY attempt_index ASC",
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
    assert_eq!(attempt_rows[2], (3, 2, 1, None));
    let route_account_id =
        wait_for_test_sticky_route_account_id(&state.pool, "sticky-429-mixed-budget")
            .await
            .expect("sticky route should stay on primary account");
    assert_ne!(route_account_id, primary_id);
}

#[test]
fn pool_route_switches_accounts_immediately_after_upstream_402() {
    run_routing_failover_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) =
            spawn_pool_static_failure_responses_upstream(&[(
                "Bearer upstream-primary",
                StatusCode::PAYMENT_REQUIRED,
            )])
            .await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let primary_id =
            insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
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

        {
            let attempts = attempts.lock().expect("lock attempts");
            assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
            assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
        }

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
    });
}
