#[tokio::test]
pub(crate) async fn api_key_model_route_health_isolated_and_resettable() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Model health account",
        "model-health-key",
        None,
        Some("https://model-health.example.com/backend-api/codex"),
    )
    .await;

    async fn attempt(state: &AppState, account_id: i64, model: &str) -> i64 {
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, status) VALUES (?1, ?2, '/v1/responses', 'pool', ?3, ?4, 'route', 1, 1, 0, 'failed')",
        )
        .bind(format!("model-health-{model}"))
        .bind(format_utc_iso(Utc::now()))
        .bind(model)
        .bind(account_id)
        .execute(&state.pool)
        .await
        .expect("insert model attempt")
        .last_insert_rowid()
    }

    let attempt_a = attempt(&state, account_id, "gpt-5.5").await;
    for _ in 0..5 {
        record_model_route_failure_from_attempt(
            &state.pool,
            account_id,
            attempt_a,
            StatusCode::BAD_REQUEST,
            Some("unsupported model: gpt-5.5"),
            Some("model_unavailable"),
        )
        .await
        .expect("record model-specific failure");
    }
    let states = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load model routing states");
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].model, "gpt-5.5");
    assert_eq!(states[0].state, MODEL_ROUTE_STATE_COOLING_DOWN);
    assert_eq!(states[0].priority, MODEL_ROUTE_PRIORITY_EXCLUDED);
    assert_eq!(
        model_route_penalty(&state.pool, account_id, Some("gpt-5.5"))
            .await
            .unwrap(),
        ModelRoutePenalty::Excluded
    );

    let attempt_b = attempt(&state, account_id, "gpt-5.4").await;
    record_model_route_success_from_attempt(&state.pool, account_id, attempt_b, None)
        .await
        .expect("record independent model success");
    assert_eq!(
        model_route_penalty(&state.pool, account_id, Some("gpt-5.4"))
            .await
            .unwrap(),
        ModelRoutePenalty::Normal
    );

    reset_model_route(&state.pool, account_id, "gpt-5.5")
        .await
        .expect("reset model route")
        .expect("model route exists");
    let reset = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load reset state")
        .into_iter()
        .find(|item| item.model == "gpt-5.5")
        .expect("reset model state exists");
    assert_eq!(reset.state, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(reset.priority, MODEL_ROUTE_PRIORITY_NORMAL);
    assert_eq!(reset.failure_count, 0);
    let event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_account_events WHERE account_id = ?1 AND model = 'gpt-5.5'",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("count model events");
    assert!(event_count >= 2);
}

#[tokio::test]
pub(crate) async fn concurrent_model_failures_reach_cooldown_threshold_without_lost_updates() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Concurrent model health",
        "concurrent-model-health-key",
        None,
        Some("https://concurrent-model-health.example.com/backend-api/codex"),
    )
    .await;
    let attempt_id = sqlx::query(
        "INSERT INTO pool_upstream_request_attempts (invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, status) VALUES ('concurrent-model-health', ?1, '/v1/responses', 'pool', 'gpt-concurrent', ?2, 'route', 1, 1, 0, 'failed')",
    )
    .bind(format_utc_iso(Utc::now()))
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("insert concurrent model attempt")
    .last_insert_rowid();

    let failure = || {
        record_model_route_failure_from_attempt(
            &state.pool,
            account_id,
            attempt_id,
            StatusCode::BAD_REQUEST,
            Some("model unavailable"),
            Some("model_unavailable"),
        )
    };
    let results = tokio::join!(failure(), failure(), failure(), failure(), failure());
    for result in [results.0, results.1, results.2, results.3, results.4] {
        result.expect("record concurrent model failure");
    }

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load concurrent route")
        .into_iter()
        .find(|state| state.model == "gpt-concurrent")
        .expect("concurrent route exists");
    assert_eq!(route.failure_count, MODEL_ROUTE_FAILURE_THRESHOLD);
    assert_eq!(route.state, MODEL_ROUTE_STATE_COOLING_DOWN);
    assert_eq!(route.priority, MODEL_ROUTE_PRIORITY_EXCLUDED);
}

#[tokio::test]
pub(crate) async fn sparse_model_failures_keep_count_until_cooldown_threshold() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Sparse model health",
        "sparse-model-health-key",
        None,
        Some("https://sparse-model-health.example.com/backend-api/codex"),
    )
    .await;
    let old = format_utc_iso(Utc::now() - ChronoDuration::seconds(45));
    sqlx::query(
        "INSERT INTO pool_upstream_account_model_routes (account_id, model, state, priority, consecutive_failures, streak_started_at, changed_at, last_seen_at, last_failure_at, last_failure_kind, last_failure_message) VALUES (?1, 'gpt-sparse', 'degraded', 'demoted', 4, ?2, ?2, ?3, ?2, 'model_unavailable', 'model unavailable')",
    )
    .bind(account_id)
    .bind(&old)
    .bind(format_utc_iso(Utc::now()))
    .execute(&state.pool)
    .await
    .expect("seed sparse model route");
    let attempt_id = sqlx::query(
        "INSERT INTO pool_upstream_request_attempts (invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, status) VALUES ('sparse-model-health', ?1, '/v1/responses', 'pool', 'gpt-sparse', ?2, 'route', 1, 1, 0, 'failed')",
    )
    .bind(format_utc_iso(Utc::now()))
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("insert sparse model attempt")
    .last_insert_rowid();

    record_model_route_failure_from_attempt(
        &state.pool,
        account_id,
        attempt_id,
        StatusCode::BAD_REQUEST,
        Some("model unavailable"),
        Some("model_unavailable"),
    )
    .await
    .expect("record sparse model failure");

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load sparse model route")
        .into_iter()
        .find(|route| route.model == "gpt-sparse")
        .expect("sparse model route exists");
    assert_eq!(route.failure_count, MODEL_ROUTE_FAILURE_THRESHOLD);
    assert_eq!(route.state, MODEL_ROUTE_STATE_COOLING_DOWN);
    assert_eq!(route.priority, MODEL_ROUTE_PRIORITY_EXCLUDED);
}

#[tokio::test]
pub(crate) async fn expired_model_cooldown_reports_transition_at_expiry() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Expired model health",
        "expired-model-health-key",
        None,
        Some("https://expired-model-health.example.com/backend-api/codex"),
    )
    .await;
    let cooldown_until = format_utc_iso(Utc::now() - ChronoDuration::seconds(5));
    let changed_at = format_utc_iso(Utc::now() - ChronoDuration::seconds(20));
    sqlx::query(
        "INSERT INTO pool_upstream_account_model_routes (account_id, model, state, priority, consecutive_failures, changed_at, last_seen_at, last_failure_at, cooldown_until) VALUES (?1, 'gpt-expired', 'cooling_down', 'excluded', 5, ?2, ?3, ?2, ?4)",
    )
    .bind(account_id)
    .bind(&changed_at)
    .bind(format_utc_iso(Utc::now()))
    .bind(&cooldown_until)
    .execute(&state.pool)
    .await
    .expect("seed expired model route");

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load expired model route")
        .into_iter()
        .find(|route| route.model == "gpt-expired")
        .expect("expired model route exists");
    assert_eq!(route.state, MODEL_ROUTE_STATE_DEGRADED);
    assert_eq!(route.priority, MODEL_ROUTE_PRIORITY_DEMOTED);
    assert_eq!(
        route.changed_at.as_deref().and_then(parse_to_utc_datetime),
        parse_to_utc_datetime(&cooldown_until),
    );
    assert!(route.cooldown_until.is_none());
}

#[tokio::test]
pub(crate) async fn current_quota_route_failure_survives_informational_account_updates() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Quota exhausted after edit").await;

    record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            false,
            Some("sticky-quota-after-edit"),
            StatusCode::TOO_MANY_REQUESTS,
            "oauth_upstream_rejected_request: pool upstream responded with 429: The usage limit has been reached",
            Some("invk_quota_after_edit"),
        )
        .await
        .expect("record wrapped 429 route failure before edit");

    record_account_update_action(
        &pool,
        account_id,
        "account settings were updated after the quota-exhausted failure",
    )
    .await
    .expect("record account update action");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load updated row")
        .expect("updated row exists");
    let summary = build_summary_from_row(
        &row,
        None,
        row.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );

    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(
        summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(
        summary.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_ACCOUNT_UPDATED)
    );
    assert_eq!(
        row.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
}

#[tokio::test]
pub(crate) async fn oauth_summary_exports_missing_refresh_token_flag() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Manual RT omitted").await;
    sqlx::query("UPDATE pool_upstream_accounts SET has_refresh_token = 0 WHERE id = ?")
        .bind(account_id)
        .execute(&pool)
        .await
        .expect("mark account as missing refresh token");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load no refresh token row")
        .expect("no refresh token row exists");
    let summary = build_summary_from_row(
        &row,
        None,
        row.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );

    assert!(!summary.has_refresh_token);
}

pub(crate) async fn insert_limit_sample(
    pool: &SqlitePool,
    account_id: i64,
    captured_at: &str,
    plan_type: Option<&str>,
) {
    sqlx::query(
        r#"
            INSERT INTO pool_upstream_account_limit_samples (
                account_id, captured_at, limit_id, limit_name, plan_type,
                primary_used_percent, primary_window_minutes, primary_resets_at,
                secondary_used_percent, secondary_window_minutes, secondary_resets_at,
                credits_has_credits, credits_unlimited, credits_balance
            ) VALUES (
                ?1, ?2, NULL, NULL, ?3,
                NULL, NULL, NULL,
                NULL, NULL, NULL,
                NULL, NULL, NULL
            )
            "#,
    )
    .bind(account_id)
    .bind(captured_at)
    .bind(plan_type)
    .execute(pool)
    .await
    .expect("insert limit sample");
}

pub(crate) async fn insert_limit_sample_with_usage(
    pool: &SqlitePool,
    account_id: i64,
    captured_at: &str,
    primary_used_percent: Option<f64>,
    secondary_used_percent: Option<f64>,
) {
    sqlx::query(
        r#"
            INSERT INTO pool_upstream_account_limit_samples (
                account_id, captured_at, limit_id, limit_name, plan_type,
                primary_used_percent, primary_window_minutes, primary_resets_at,
                secondary_used_percent, secondary_window_minutes, secondary_resets_at,
                credits_has_credits, credits_unlimited, credits_balance
            ) VALUES (
                ?1, ?2, NULL, NULL, 'team',
                ?3, 300, NULL,
                ?4, 10080, NULL,
                NULL, NULL, NULL
            )
            "#,
    )
    .bind(account_id)
    .bind(captured_at)
    .bind(primary_used_percent)
    .bind(secondary_used_percent)
    .execute(pool)
    .await
    .expect("insert limit sample with usage");
}

pub(crate) async fn insert_limit_sample_with_reset_times(
    pool: &SqlitePool,
    account_id: i64,
    captured_at: &str,
    primary_resets_at: Option<&str>,
    secondary_resets_at: Option<&str>,
    primary_used_percent: f64,
    secondary_used_percent: f64,
) {
    sqlx::query(
        r#"
            INSERT INTO pool_upstream_account_limit_samples (
                account_id, captured_at, limit_id, limit_name, plan_type,
                primary_used_percent, primary_window_minutes, primary_resets_at,
                secondary_used_percent, secondary_window_minutes, secondary_resets_at,
                credits_has_credits, credits_unlimited, credits_balance
            ) VALUES (
                ?1, ?2, NULL, NULL, 'team',
                ?3, 300, ?4,
                ?5, 10080, ?6,
                NULL, NULL, NULL
            )
            "#,
    )
    .bind(account_id)
    .bind(captured_at)
    .bind(primary_used_percent)
    .bind(primary_resets_at)
    .bind(secondary_used_percent)
    .bind(secondary_resets_at)
    .execute(pool)
    .await
    .expect("insert limit sample with reset times");
}

pub(crate) async fn seed_route_cooldown(
    pool: &SqlitePool,
    account_id: i64,
    failure_kind: &str,
    cooldown_secs: i64,
) {
    let now = Utc::now();
    let now_iso = format_utc_iso(now);
    let cooldown_until = format_utc_iso(now + ChronoDuration::seconds(cooldown_secs));
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET status = ?2,
                last_error = ?3,
                last_error_at = ?4,
                last_route_failure_at = ?4,
                last_route_failure_kind = ?5,
                cooldown_until = ?6,
                consecutive_route_failures = 1,
                temporary_route_failure_streak_started_at = NULL,
                updated_at = ?4
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind("seed route cooldown")
    .bind(&now_iso)
    .bind(failure_kind)
    .bind(&cooldown_until)
    .execute(pool)
    .await
    .expect("seed route cooldown");
}

#[test]
pub(crate) fn pool_blocked_failure_kinds_are_not_temporary_route_failures() {
    assert!(!route_failure_kind_is_temporary(Some(
        PROXY_FAILURE_POOL_ROUTING_BLOCKED,
    )));
    assert!(!route_failure_kind_is_temporary(Some(
        PROXY_FAILURE_POOL_ASSIGNED_ACCOUNT_BLOCKED,
    )));
}

pub(crate) async fn seed_hard_unavailable_route_failure(
    pool: &SqlitePool,
    account_id: i64,
    status: &str,
    failure_kind: &str,
    reason_code: &str,
    http_status: Option<i64>,
) {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET status = ?2,
                last_error = ?3,
                last_error_at = ?4,
                last_route_failure_at = ?4,
                last_route_failure_kind = ?5,
                cooldown_until = NULL,
                consecutive_route_failures = 1,
                temporary_route_failure_streak_started_at = NULL,
                last_action = ?6,
                last_action_source = ?7,
                last_action_reason_code = ?8,
                last_action_reason_message = ?3,
                last_action_http_status = ?9,
                last_action_at = ?4,
                updated_at = ?4
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind(status)
    .bind("seed hard unavailable")
    .bind(&now_iso)
    .bind(failure_kind)
    .bind(UPSTREAM_ACCOUNT_ACTION_ROUTE_HARD_UNAVAILABLE)
    .bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL)
    .bind(reason_code)
    .bind(http_status)
    .execute(pool)
    .await
    .expect("seed hard unavailable");
}

#[tokio::test]
pub(crate) async fn record_pool_route_success_does_not_clear_newer_route_failure_state() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Stale Success Guard").await;
    seed_hard_unavailable_route_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    record_pool_route_success(
        &pool,
        account_id,
        Utc::now() - ChronoDuration::minutes(5),
        Some("sticky-stale-success"),
        Some("invk_stale_success"),
    )
    .await
    .expect("record stale route success");

    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after stale success")
        .expect("row exists after stale success");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_ROUTE_HARD_UNAVAILABLE)
    );
    assert!(
        load_sticky_route(&pool, "sticky-stale-success")
            .await
            .expect("load sticky route after stale success")
            .is_none()
    );
}

#[tokio::test]
pub(crate) async fn model_success_is_recorded_when_account_success_is_stale() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Stale Account Success Model Recovery").await;
    let request_started_at_utc = Utc::now() - ChronoDuration::seconds(5);
    let request_started_at = format_naive_precise(
        request_started_at_utc
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let attempt_id = sqlx::query(
        "INSERT INTO pool_upstream_request_attempts (invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, started_at, status) VALUES ('stale-account-success-model', ?1, '/v1/responses', 'pool', 'gpt-stale-account-success', ?2, 'route', 1, 1, 0, ?3, 'failed')",
    )
    .bind(format_utc_iso(Utc::now()))
    .bind(account_id)
    .bind(&request_started_at)
    .execute(&pool)
    .await
    .expect("insert stale account success attempt")
    .last_insert_rowid();

    let model_failure_at = format_naive_precise(
        (request_started_at_utc - ChronoDuration::seconds(1))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    sqlx::query(
        "INSERT INTO pool_upstream_account_model_routes (account_id, model, state, priority, consecutive_failures, streak_started_at, changed_at, last_seen_at, last_failure_at, last_failure_kind, last_failure_message) VALUES (?1, 'gpt-stale-account-success', 'degraded', 'demoted', 1, ?2, ?2, ?3, ?2, 'model_unavailable', 'model unavailable')",
    )
    .bind(account_id)
    .bind(&model_failure_at)
    .bind(format_utc_iso(Utc::now()))
    .execute(&pool)
    .await
    .expect("seed stale account success model failure");
    seed_hard_unavailable_route_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    record_pool_route_success_for_endpoint_with_image_intent_for_attempt(
        &pool,
        account_id,
        request_started_at_utc,
        None,
        None,
        "/v1/responses",
        ImageIntent::Unknown,
        Some(attempt_id),
    )
    .await
    .expect("record stale account success");

    let route = load_model_routing_states(&pool, account_id)
        .await
        .expect("load stale account success model route")
        .into_iter()
        .find(|route| route.model == "gpt-stale-account-success")
        .expect("stale account success model route exists");
    assert_eq!(route.state, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(route.priority, MODEL_ROUTE_PRIORITY_NORMAL);
    assert_eq!(route.failure_count, 0);
    assert!(route.last_failure_at.is_none());

    let account = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load stale account success account")
        .expect("stale account success account exists");
    assert_eq!(account.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
}

#[tokio::test]
pub(crate) async fn image_intent_route_success_learns_supported_capability() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Image Success Learns Supported").await;

    record_pool_route_success_for_endpoint_with_image_intent(
        &pool,
        account_id,
        Utc::now(),
        Some("sticky-image-supported"),
        Some("invk_image_supported"),
        "/v1/responses",
        ImageIntent::Yes,
    )
    .await
    .expect("record image route success");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after image success")
        .expect("row exists after image success");
    assert_eq!(
        row.response_endpoint_capability.as_deref(),
        Some("supported")
    );
    assert_eq!(
        row.response_image_tool_capability.as_deref(),
        Some("supported")
    );
    assert_eq!(row.chat_completions_capability.as_deref(), Some("unknown"));

    let direct_account_id =
        insert_oauth_account(&pool, "Direct Image Success Learns Supported").await;
    record_pool_route_success_for_endpoint_with_image_intent(
        &pool,
        direct_account_id,
        Utc::now(),
        Some("sticky-direct-image-supported"),
        Some("invk_direct_image_supported"),
        "/v1/images/generations",
        ImageIntent::DirectImage,
    )
    .await
    .expect("record direct image route success");

    let direct_row = load_upstream_account_row(&pool, direct_account_id)
        .await
        .expect("load row after direct image success")
        .expect("row exists after direct image success");
    assert_eq!(
        direct_row.image_endpoint_capability.as_deref(),
        Some("supported")
    );
    assert_eq!(
        direct_row.response_endpoint_capability.as_deref(),
        Some("unknown")
    );
}

#[tokio::test]
pub(crate) async fn chat_completions_route_learning_stays_on_chat_axis() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Chat Success Learns Supported").await;

    record_pool_route_success_for_endpoint_with_image_intent(
        &pool,
        account_id,
        Utc::now(),
        Some("sticky-chat-supported"),
        Some("invk_chat_supported"),
        "/v1/chat/completions",
        ImageIntent::No,
    )
    .await
    .expect("record chat completions route success");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after chat success")
        .expect("row exists after chat success");
    assert_eq!(
        row.chat_completions_capability.as_deref(),
        Some("supported")
    );
    assert_eq!(row.response_endpoint_capability.as_deref(), Some("unknown"));
    assert_eq!(row.image_endpoint_capability.as_deref(), Some("unknown"));

    let unsupported_account_id =
        insert_oauth_account(&pool, "Chat Failure Learns Unsupported").await;
    record_pool_route_http_failure_for_endpoint_with_image_intent(
        &pool,
        unsupported_account_id,
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        false,
        Some("sticky-chat-unsupported"),
        StatusCode::NOT_FOUND,
        "pool upstream responded with 404: unsupported endpoint /v1/chat/completions for this account",
        Some("invk_chat_unsupported"),
        "/v1/chat/completions",
        ImageIntent::No,
    )
    .await
    .expect("record explicit unsupported chat completions failure");

    let unsupported_row = load_upstream_account_row(&pool, unsupported_account_id)
        .await
        .expect("load row after unsupported chat failure")
        .expect("row exists after unsupported chat failure");
    assert_eq!(
        unsupported_row.chat_completions_capability.as_deref(),
        Some("unsupported")
    );
    assert_eq!(
        unsupported_row.response_endpoint_capability.as_deref(),
        Some("unknown")
    );
}

#[tokio::test]
pub(crate) async fn standalone_search_learning_is_api_key_only_and_route_specific() {
    let pool = test_pool().await;
    let supported_id = insert_api_key_account(&pool, "Search Success Learns Supported").await;
    record_pool_route_success_for_endpoint_with_image_intent(
        &pool,
        supported_id,
        Utc::now(),
        None,
        Some("invk_search_supported"),
        "/v1/alpha/search",
        ImageIntent::No,
    )
    .await
    .expect("record search route success");
    let supported = load_upstream_account_row(&pool, supported_id)
        .await
        .expect("load supported search row")
        .expect("supported search row exists");
    assert_eq!(
        supported.standalone_search_capability.as_deref(),
        Some("supported")
    );
    assert_eq!(
        supported.response_endpoint_capability.as_deref(),
        Some("unknown")
    );

    let unsupported_id = insert_api_key_account(&pool, "Search 404 Learns Unsupported").await;
    record_pool_route_http_failure_for_endpoint_with_image_intent(
        &pool,
        unsupported_id,
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
        false,
        None,
        StatusCode::NOT_FOUND,
        "pool upstream responded with 404",
        Some("invk_search_unsupported"),
        "/v1/alpha/search",
        ImageIntent::No,
    )
    .await
    .expect("record bare search 404");
    let unsupported = load_upstream_account_row(&pool, unsupported_id)
        .await
        .expect("load unsupported search row")
        .expect("unsupported search row exists");
    assert_eq!(
        unsupported.standalone_search_capability.as_deref(),
        Some("unsupported")
    );

    let ambiguous_id = insert_api_key_account(&pool, "Search 400 Keeps Unknown").await;
    record_pool_route_http_failure_for_endpoint_with_image_intent(
        &pool,
        ambiguous_id,
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
        false,
        None,
        StatusCode::BAD_REQUEST,
        "pool upstream responded with 400: request body is invalid",
        Some("invk_search_ambiguous"),
        "/v1/alpha/search",
        ImageIntent::No,
    )
    .await
    .expect("record ambiguous search 400");
    let ambiguous = load_upstream_account_row(&pool, ambiguous_id)
        .await
        .expect("load ambiguous search row")
        .expect("ambiguous search row exists");
    assert_eq!(
        ambiguous.standalone_search_capability.as_deref(),
        Some("unknown")
    );

    let oauth_id = insert_oauth_account(&pool, "OAuth Search Does Not Learn").await;
    record_pool_route_success_for_endpoint_with_image_intent(
        &pool,
        oauth_id,
        Utc::now(),
        None,
        Some("invk_oauth_search"),
        "/v1/alpha/search",
        ImageIntent::No,
    )
    .await
    .expect("record OAuth search route success");
    let oauth = load_upstream_account_row(&pool, oauth_id)
        .await
        .expect("load OAuth search row")
        .expect("OAuth search row exists");
    assert_eq!(
        oauth.standalone_search_capability.as_deref(),
        Some("unknown")
    );
}

#[tokio::test]
pub(crate) async fn standalone_search_override_round_trips_through_account_detail() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Search Override Round Trip").await;
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET standalone_search_capability = 'unsupported',
            standalone_search_capability_observed_at = '2026-08-05T15:00:00Z',
            standalone_search_capability_reason = 'pool upstream responded with 404'
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("seed unsupported search capability");

    let detail = state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            account_id,
            UpdateUpstreamAccountRequest {
                standalone_search_capability_override: OptionalField::Value(
                    "supported".to_string(),
                ),
                ..UpdateUpstreamAccountRequest::default()
            },
        )
        .await
        .expect("save search capability override");

    assert_eq!(
        detail.summary.standalone_search_capability.observed,
        CapabilitySupport::Unsupported
    );
    assert_eq!(
        detail.summary.standalone_search_capability.override_value,
        Some(CapabilitySupport::Supported)
    );
    assert_eq!(
        detail.summary.standalone_search_capability.effective,
        CapabilitySupport::Supported
    );
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load account after search override")
        .expect("account exists after search override");
    assert_eq!(
        row.policy_standalone_search_capability_override.as_deref(),
        Some("supported")
    );

    let detail = state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            account_id,
            UpdateUpstreamAccountRequest {
                standalone_search_capability_override: OptionalField::Null,
                ..UpdateUpstreamAccountRequest::default()
            },
        )
        .await
        .expect("clear search capability override");

    assert_eq!(
        detail.summary.standalone_search_capability.override_value,
        None
    );
    assert_eq!(
        detail.summary.standalone_search_capability.effective,
        CapabilitySupport::Unsupported
    );
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load account after clearing search override")
        .expect("account exists after clearing search override");
    assert_eq!(row.policy_standalone_search_capability_override, None);
}

use super::*;
