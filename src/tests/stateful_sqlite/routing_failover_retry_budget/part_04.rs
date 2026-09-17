type AttemptStatusRow = (String, Option<String>);

async fn reserve_only_model_slot(state: &Arc<AppState>, account_id: i64, model: &str) {
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed model route");
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET cache_concurrency_limit = 1 WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .execute(&state.pool)
    .await
    .expect("limit model route to one reservation");
    sqlx::query(
        "UPDATE pool_routing_settings SET cache_hit_protection_enabled = 1, cache_hit_overflow_mode = 'queue' WHERE id = 1",
    )
    .execute(&state.pool)
    .await
    .expect("enable queue overflow mode");
    let holder = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        state, None, Some(model), &[], &HashSet::new(), None, None, None, "/v1/responses",
        crate::ImageIntent::Unknown, false, Some("model-reservation-holder"),
    )
    .await
    .expect("reserve the only model slot");
    assert!(matches!(holder, PoolAccountResolution::Resolved(_)));
}

#[tokio::test]
pub(crate) async fn orphan_recovery_persists_route_failure_before_releasing_reservation() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id =
        insert_test_pool_oauth_account(&state, "Orphan Failure Fence", "orphan-fence-token").await;
    let invoke_id = "proxy-98765-orphan-failure-fence";
    let reservation_key = pool_routing_reservation_key_for_invoke_id(invoke_id)
        .expect("legacy proxy invoke id should map to its reservation");
    state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned")
        .insert(
            reservation_key.clone(),
            PoolRoutingReservation {
                account_id,
                model: None,
                proxy_key: None,
                created_at: Instant::now(),
            },
        );
    let availability = state.pool_routing_availability.subscribe();
    let initial_generation = *availability.borrow();

    clean_up_pool_route_after_orphan_recovery(
        state.as_ref(),
        invoke_id,
        None,
        Some(account_id),
        "test",
        true,
    )
    .await;

    let failure_at: Option<String> = sqlx::query_scalar(
        "SELECT last_route_failure_at FROM pool_upstream_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load orphan recovery route failure");
    assert!(
        failure_at.is_some(),
        "orphan cleanup must commit the route failure before making the slot available"
    );
    assert!(
        !state
            .pool_routing_reservations
            .lock()
            .expect("pool routing reservations mutex poisoned")
            .contains_key(&reservation_key),
        "orphan cleanup should release only after the failure write returns"
    );
    assert_ne!(
        *availability.borrow(),
        initial_generation,
        "reservation release should notify waiting routing requests"
    );
}

#[tokio::test]
pub(crate) async fn resolve_pool_account_for_request_with_wait_accepts_recovery_after_wait_starts()
{
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        Duration::from_secs(2),
        Duration::from_millis(100),
    )
    .await;
    let blocked_id = insert_test_pool_api_key_account(&state, "Blocked", "upstream-blocked").await;
    let delayed_id = insert_test_pool_api_key_account(&state, "Delayed", "upstream-delayed").await;
    set_test_account_status(&state.pool, blocked_id, "needs_reauth").await;
    set_test_account_status(&state.pool, delayed_id, "needs_reauth").await;

    let wait_started_rx = crate::proxy::register_pool_no_available_wait_hook(&state);
    let pool = state.pool.clone();
    let runtime_handle = tokio::runtime::Handle::current();
    let delayed_release_task = std::thread::spawn(move || {
        wait_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("helper should signal once the bounded wait starts");
        runtime_handle.block_on(async move {
            set_test_account_status(&pool, delayed_id, "active").await;
        });
    });

    let started = Instant::now();
    let mut wait_deadline = None;
    let mut options = PoolAccountWaitOptions {
        sticky_key: None,
        requested_model: None,
        excluded_ids: &[],
        excluded_upstream_route_keys: &HashSet::new(),
        required_upstream_route_key: None,
        wait_for_no_available: true,
        wait_deadline: &mut wait_deadline,
        total_timeout_deadline: Some(Instant::now() + Duration::from_secs(5)),
    };
    let resolution = resolve_pool_account_for_request_with_wait(state.as_ref(), &mut options)
        .await
        .expect("helper resolution should succeed");
    let elapsed = started.elapsed();

    delayed_release_task
        .join()
        .expect("delayed release thread should join");

    assert!(
        elapsed < Duration::from_millis(5_500),
        "helper should still resolve once the account recovers after the bounded wait begins, elapsed={elapsed:?}"
    );
    match resolution {
        PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::Resolved(account)) => {
            assert_eq!(account.account_id, delayed_id);
            assert_eq!(
                account.auth.authorization_header_value(),
                Some("Bearer upstream-delayed")
            );
        }
        other => panic!("expected post-wait recovery to succeed, got {other:?}"),
    }
    assert!(
        options.wait_deadline.is_some(),
        "bounded waits should record the deadline once they actually start"
    );
}

// The account-creation helper drives work through `Handle::block_on` on a
// separate thread after the waiter subscribes. A current-thread runtime cannot
// make progress while this test synchronously joins that helper.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
pub(crate) async fn resolve_pool_account_for_request_with_wait_wakes_when_a_routable_account_is_created()
 {
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        Duration::from_secs(2),
        Duration::from_millis(100),
    )
    .await;
    let wait_started_rx = crate::proxy::register_pool_no_available_wait_hook(&state);
    let create_state = state.clone();
    let runtime_handle = tokio::runtime::Handle::current();
    let (account_id_tx, account_id_rx) = std::sync::mpsc::channel();
    let create_task = std::thread::spawn(move || {
        wait_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("waiter should subscribe before account creation");
        let account_id = runtime_handle.block_on(async move {
            insert_test_pool_api_key_account(
                &create_state,
                "Created During Wait",
                "upstream-created-during-wait",
            )
            .await
        });
        account_id_tx
            .send(account_id)
            .expect("send created account id");
    });

    let started = Instant::now();
    let mut wait_deadline = None;
    let mut options = PoolAccountWaitOptions {
        sticky_key: None,
        requested_model: None,
        excluded_ids: &[],
        excluded_upstream_route_keys: &HashSet::new(),
        required_upstream_route_key: None,
        wait_for_no_available: true,
        wait_deadline: &mut wait_deadline,
        total_timeout_deadline: Some(Instant::now() + Duration::from_secs(1)),
    };
    let resolution = resolve_pool_account_for_request_with_wait(state.as_ref(), &mut options)
        .await
        .expect("waiter should resolve after account creation publishes availability");
    let elapsed = started.elapsed();
    create_task
        .join()
        .expect("account creation task should join");
    let account_id = account_id_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("receive created account id");

    match resolution {
        PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::Resolved(account)) => {
            assert_eq!(account.account_id, account_id);
        }
        other => panic!("created routable account should resolve, got {other:?}"),
    }
    assert!(
        elapsed < Duration::from_millis(800),
        "account creation should wake the waiter before its deadline, elapsed={elapsed:?}"
    );
}

#[tokio::test]
pub(crate) async fn resolve_pool_account_for_request_with_wait_wakes_when_model_reservation_is_released()
 {
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        Duration::from_secs(2),
        Duration::from_millis(100),
    )
    .await;
    let account_id =
        insert_test_pool_api_key_account(&state, "Model Limited", "upstream-model-limited").await;
    let model = "gpt-model-reservation-wake";
    reserve_only_model_slot(&state, account_id, model).await;

    let wait_started_rx = crate::proxy::register_pool_no_available_wait_hook(&state);
    let release_state = state.clone();
    let runtime_handle = tokio::runtime::Handle::current();
    let release_task = std::thread::spawn(move || {
        wait_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("waiter should subscribe before release");
        runtime_handle.block_on(async move {
            persist_pool_route_success_then_release(
                release_state.as_ref(),
                "model-reservation-holder",
                async { Ok::<bool, ()>(true) },
            )
            .await
            .expect("healthy reservation release should persist");
        });
    });

    let started = Instant::now();
    let mut wait_deadline = None;
    let resolution = resolve_pool_account_for_request_with_wait_and_binding_constraint_with_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        state.as_ref(),
        None,
        Some(model),
        &[],
        &HashSet::new(),
        None,
        None,
        None,
        true,
        &mut wait_deadline,
        Some(Instant::now() + Duration::from_secs(1)),
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("model-reservation-waiter"),
    )
    .await
    .expect("waiter should resolve after the availability event");
    let elapsed = started.elapsed();
    release_task.join().expect("release task should join");

    match resolution {
        PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::Resolved(account)) => {
            assert_eq!(account.account_id, account_id);
        }
        other => panic!("expected released model slot to resolve, got {other:?}"),
    }
    assert!(
        elapsed < Duration::from_millis(500),
        "reservation release should wake the waiter before the two-second queue timeout, elapsed={elapsed:?}"
    );

    release_pool_routing_reservation(&state, "model-reservation-waiter");
}

#[tokio::test]
pub(crate) async fn expired_model_cooldown_probe_conflict_has_a_distinct_no_candidate_reason() {
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        Duration::from_millis(80),
        Duration::from_millis(10),
    )
    .await;
    let account_id =
        insert_test_pool_api_key_account(&state, "Cooldown Probe", "upstream-probe").await;
    let model = "gpt-expired-cooldown-probe";
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed model route");
    sqlx::query(
        r#"
        UPDATE pool_upstream_account_model_routes
           SET state = 'cooling_down',
               priority = 'excluded',
               cooldown_until = ?3,
               last_failure_kind = 'upstream_http_503'
         WHERE account_id = ?1 AND model = ?2
        "#,
    )
    .bind(account_id)
    .bind(model)
    .bind(format_utc_iso(Utc::now() - ChronoDuration::seconds(1)))
    .execute(&state.pool)
    .await
    .expect("expire model cooldown");
    sqlx::query("UPDATE pool_routing_settings SET cache_hit_overflow_mode = 'queue' WHERE id = 1")
        .execute(&state.pool)
        .await
        .expect("enable queue overflow mode");

    let holder = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        None,
        Some(model),
        &[],
        &HashSet::new(),
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("expired-probe-holder"),
    )
    .await
    .expect("reserve expired cooldown probe");
    assert!(matches!(holder, PoolAccountResolution::Resolved(_)));

    let conflict = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        None,
        Some(model),
        &[],
        &HashSet::new(),
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("expired-probe-waiter"),
    )
    .await
    .expect("resolve occupied expired cooldown probe");
    let PoolAccountResolution::NoCandidate(audit) = conflict else {
        panic!("occupied expired cooldown probe should return NoCandidate");
    };
    assert_eq!(audit.terminal_reason_code, "expiredCooldownProbe");
    assert_eq!(audit.reservation_conflict_count, 1);
    assert_eq!(audit.excluded_reason_counts["expiredCooldownProbe"], 1);
    assert_eq!(audit.candidates[0].reason_code, "expiredCooldownProbe");

    release_pool_routing_reservation(&state, "expired-probe-holder");
}

#[tokio::test]
pub(crate) async fn queued_model_capacity_audit_counts_every_conflicting_candidate() {
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        Duration::from_millis(80),
        Duration::from_millis(10),
    )
    .await;
    let first_id = insert_test_pool_api_key_account(&state, "Capacity One", "capacity-one").await;
    let second_id = insert_test_pool_api_key_account(&state, "Capacity Two", "capacity-two").await;
    let model = "gpt-queued-capacity-audit";
    for account_id in [first_id, second_id] {
        observe_model_route_seen(&state.pool, account_id, Some(model))
            .await
            .expect("seed model route");
        sqlx::query(
            "UPDATE pool_upstream_account_model_routes SET cache_concurrency_limit = 1, cache_recovery_limit = 2 WHERE account_id = ?1 AND model = ?2",
        )
        .bind(account_id)
        .bind(model)
        .execute(&state.pool)
        .await
        .expect("limit model route capacity");
    }
    sqlx::query(
        "UPDATE pool_routing_settings SET cache_hit_protection_enabled = 1, cache_hit_overflow_mode = 'queue' WHERE id = 1",
    )
    .execute(&state.pool)
    .await
    .expect("enable queue overflow mode");
    {
        let mut reservations = state
            .pool_routing_reservations
            .lock()
            .expect("pool routing reservations mutex poisoned");
        for (key, account_id) in [
            ("capacity-holder-one", first_id),
            ("capacity-holder-two", second_id),
        ] {
            reservations.insert(
                key.to_string(),
                PoolRoutingReservation {
                    account_id,
                    model: Some(model.to_string()),
                    proxy_key: None,
                    created_at: Instant::now(),
                },
            );
        }
    }

    let resolution = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        None,
        Some(model),
        &[],
        &HashSet::new(),
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("capacity-audit-waiter"),
    )
    .await
    .expect("resolve queued capacity audit");
    let PoolAccountResolution::NoCandidate(audit) = resolution else {
        panic!("all occupied model routes should return NoCandidate");
    };
    assert_eq!(audit.terminal_reason_code, "modelConcurrencyLimit");
    assert_eq!(audit.eligible_candidate_count, 2);
    assert_eq!(audit.reservation_conflict_count, 2);
    assert_eq!(audit.excluded_reason_counts["modelConcurrencyLimit"], 2);
    assert_eq!(audit.candidates.len(), 2);
}

#[tokio::test]
pub(crate) async fn queued_sticky_capacity_audit_counts_remaining_conflicting_candidates_without_rerouting()
 {
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        Duration::from_millis(80),
        Duration::from_millis(10),
    )
    .await;
    let sticky_id =
        insert_test_pool_api_key_account(&state, "Sticky Capacity", "sticky-capacity").await;
    let other_id =
        insert_test_pool_api_key_account(&state, "Other Capacity", "other-capacity").await;
    let sticky_key = "sticky-queue-capacity-audit";
    let model = "gpt-sticky-queued-capacity-audit";
    upsert_test_sticky_route_at(&state.pool, sticky_key, sticky_id, &shanghai_now_string()).await;
    for account_id in [sticky_id, other_id] {
        observe_model_route_seen(&state.pool, account_id, Some(model))
            .await
            .expect("seed sticky audit model route");
        sqlx::query(
            "UPDATE pool_upstream_account_model_routes SET cache_concurrency_limit = 1, cache_recovery_limit = 2 WHERE account_id = ?1 AND model = ?2",
        )
        .bind(account_id)
        .bind(model)
        .execute(&state.pool)
        .await
        .expect("limit sticky audit model route capacity");
    }
    sqlx::query(
        "UPDATE pool_routing_settings SET cache_hit_protection_enabled = 1, cache_hit_overflow_mode = 'queue' WHERE id = 1",
    )
    .execute(&state.pool)
    .await
    .expect("enable queue overflow mode");
    {
        let mut reservations = state
            .pool_routing_reservations
            .lock()
            .expect("pool routing reservations mutex poisoned");
        for (key, account_id) in [
            ("sticky-capacity-holder", sticky_id),
            ("other-capacity-holder", other_id),
        ] {
            reservations.insert(
                key.to_string(),
                PoolRoutingReservation {
                    account_id,
                    model: Some(model.to_string()),
                    proxy_key: None,
                    created_at: Instant::now(),
                },
            );
        }
    }

    let resolution = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        Some(sticky_key),
        Some(model),
        &[],
        &HashSet::new(),
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("sticky-capacity-audit-waiter"),
    )
    .await
    .expect("resolve queued sticky capacity audit");
    let PoolAccountResolution::NoCandidate(audit) = resolution else {
        panic!("sticky conflict in queue mode must remain NoCandidate");
    };
    assert_eq!(audit.terminal_reason_code, "stickyRouteReservationConflict");
    assert_eq!(audit.candidate_count, 2);
    assert_eq!(audit.eligible_candidate_count, 2);
    assert_eq!(audit.reservation_conflict_count, 2);
    assert_eq!(
        audit.excluded_reason_counts["stickyRouteReservationConflict"],
        1
    );
    assert_eq!(audit.excluded_reason_counts["modelConcurrencyLimit"], 1);
    assert!(
        !state
            .pool_routing_reservations
            .lock()
            .expect("pool routing reservations mutex poisoned")
            .contains_key("sticky-capacity-audit-waiter"),
        "queue auditing must not reserve a later candidate"
    );
}

#[tokio::test]
pub(crate) async fn queued_model_capacity_audit_ignores_unrelated_cooldown_expiry() {
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        Duration::from_millis(80),
        Duration::from_millis(10),
    )
    .await;
    let capacity_id =
        insert_test_pool_api_key_account(&state, "Capacity Target", "capacity-target").await;
    let unrelated_id =
        insert_test_pool_api_key_account(&state, "Inactive Cooldown", "inactive-cooldown").await;
    let model = "gpt-unrelated-cooldown-audit";
    for account_id in [capacity_id, unrelated_id] {
        observe_model_route_seen(&state.pool, account_id, Some(model))
            .await
            .expect("seed model route");
    }
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET cache_concurrency_limit = 1, cache_recovery_limit = 2 WHERE account_id = ?1 AND model = ?2",
    )
    .bind(capacity_id)
    .bind(model)
    .execute(&state.pool)
    .await
    .expect("limit target model route capacity");
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET state = 'cooling_down', priority = 'excluded', cooldown_until = ?3 WHERE account_id = ?1 AND model = ?2",
    )
    .bind(unrelated_id)
    .bind(model)
    .bind(format_utc_iso(Utc::now() + ChronoDuration::minutes(5)))
    .execute(&state.pool)
    .await
    .expect("seed unrelated cooldown");
    set_test_account_status(&state.pool, unrelated_id, "needs_reauth").await;
    sqlx::query(
        "UPDATE pool_routing_settings SET cache_hit_protection_enabled = 1, cache_hit_overflow_mode = 'queue' WHERE id = 1",
    )
    .execute(&state.pool)
    .await
    .expect("enable queue overflow mode");
    state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned")
        .insert(
            "capacity-target-holder".to_string(),
            PoolRoutingReservation {
                account_id: capacity_id,
                model: Some(model.to_string()),
                proxy_key: None,
                created_at: Instant::now(),
            },
        );

    let resolution = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        None,
        Some(model),
        &[],
        &HashSet::new(),
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("unrelated-cooldown-waiter"),
    )
    .await
    .expect("resolve queued capacity audit");
    let PoolAccountResolution::NoCandidate(audit) = resolution else {
        panic!("occupied target route should return NoCandidate");
    };
    assert_eq!(audit.reservation_conflict_count, 1);
    assert_eq!(audit.next_eligible_at, None);
}

#[tokio::test]
pub(crate) async fn resolve_pool_account_for_request_with_wait_rejects_recovery_after_external_deadline()
 {
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
    let mut options = PoolAccountWaitOptions {
        sticky_key: None,
        requested_model: None,
        excluded_ids: &[],
        excluded_upstream_route_keys: &HashSet::new(),
        required_upstream_route_key: None,
        wait_for_no_available: true,
        wait_deadline: &mut wait_deadline,
        total_timeout_deadline: Some(Instant::now() + Duration::from_millis(40)),
    };
    let resolution = resolve_pool_account_for_request_with_wait(state.as_ref(), &mut options)
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

#[test]
pub(crate) fn pool_route_wait_timeout_overrides_stale_upstream_failure_with_503() {
    run_routing_failover_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) =
            spawn_pool_static_failure_responses_upstream(&[(
                "Bearer upstream-primary",
                StatusCode::INTERNAL_SERVER_ERROR,
            )])
            .await;
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(60),
            Duration::from_millis(10),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
        let blocked_id =
            insert_test_pool_api_key_account(&state, "Blocked", "upstream-blocked").await;
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

        {
            let attempts = attempts.lock().expect("lock attempts");
            assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
            assert_eq!(attempts.get("Bearer upstream-blocked").copied(), None);
        }

        upstream_handle.abort();
    });
}

#[test]
pub(crate) fn pool_route_existing_sticky_owner_retries_before_cutting_out_to_healthy_alternate() {
    run_routing_failover_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) =
            spawn_pool_static_failure_responses_upstream(&[(
                "Bearer upstream-primary",
                StatusCode::INTERNAL_SERVER_ERROR,
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

        {
            let attempts = attempts.lock().expect("lock attempts");
            assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
            assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
        }

        let mut route_account_id =
            load_test_sticky_route_account_id(&state.pool, "sticky-existing-owner-cutout").await;
        for _ in 0..20 {
            if route_account_id == Some(secondary_id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
            route_account_id =
                load_test_sticky_route_account_id(&state.pool, "sticky-existing-owner-cutout")
                    .await;
        }
        assert_eq!(
            route_account_id,
            Some(secondary_id),
            "sticky binding should move only after the alternate succeeds",
        );

        upstream_handle.abort();
    });
}

#[test]
pub(crate) fn pool_route_existing_sticky_owner_preserves_last_failure_when_cutout_target_is_unusable()
 {
    run_routing_failover_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) =
            spawn_pool_static_failure_responses_upstream(&[(
                "Bearer upstream-primary",
                StatusCode::INTERNAL_SERVER_ERROR,
            )])
            .await;
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(180),
            Duration::from_millis(10),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let primary_id =
            insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
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

        wait_for_pool_attempt_row_count(&state.pool, 3).await;
        assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 3);

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
        assert_eq!(attempt_rows.len(), 3);

        {
            let attempts = attempts.lock().expect("lock attempts");
            assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
            assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);
        }

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
    });
}

use super::*;
