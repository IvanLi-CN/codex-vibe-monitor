use super::*;
use serde_json::json;
use std::time::Instant;

async fn assert_successful_oauth_sync(
    state: &AppState,
    account_id: i64,
    usage_requests: &AtomicUsize,
    token_requests: &AtomicUsize,
    access_token: &str,
    refresh_token: &str,
) {
    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth account after sync")
        .expect("oauth account still exists");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_error.is_none());
    assert!(after.last_route_failure_kind.is_none());
    assert!(after.last_successful_sync_at.is_some());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED)
    );
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let decrypted = decrypt_credentials(
        crypto_key,
        after
            .encrypted_credentials
            .as_deref()
            .expect("encrypted oauth credentials"),
    )
    .expect("decrypt synced credentials");
    let StoredCredentials::Oauth(credentials) = decrypted else {
        panic!("oauth account should keep oauth credentials");
    };
    assert_eq!(credentials.access_token, access_token);
    assert_eq!(credentials.refresh_token.as_deref(), Some(refresh_token));
    assert_eq!(token_requests.load(Ordering::SeqCst), 1);
    assert_eq!(usage_requests.load(Ordering::SeqCst), 1);
}

async fn seed_manual_node_shunt_accounts(state: &Arc<AppState>) -> (i64, i64) {
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let occupying_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Occupying OAuth",
        "occupying@example.com",
        "org_occupying",
        "user_occupying",
    )
    .await;
    let queued_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Queued OAuth",
        "queued@example.com",
        "org_queued",
        "user_queued",
    )
    .await;
    set_test_account_group_name(&state.pool, occupying_account_id, Some("node-shunt-sync")).await;
    set_test_account_group_name(&state.pool, queued_account_id, Some("node-shunt-sync")).await;
    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-sync",
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
    .expect("save node shunt sync metadata");
    drop(conn);
    (occupying_account_id, queued_account_id)
}

async fn assert_node_shunt_unassigned(
    state: &AppState,
    occupying_account_id: i64,
    queued_account_id: i64,
) {
    let assignments = build_upstream_account_node_shunt_assignments(state)
        .await
        .expect("build node shunt assignments");
    assert_eq!(
        assignments
            .account_proxy_keys
            .get(&occupying_account_id)
            .map(String::as_str),
        Some(FORWARD_PROXY_DIRECT_KEY),
    );
    assert!(
        !assignments
            .account_proxy_keys
            .contains_key(&queued_account_id)
    );
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

#[tokio::test]
pub(crate) async fn standalone_search_override_is_rejected_for_oauth_accounts() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_oauth_account(&state.pool, "OAuth Search Override Rejected").await;

    let err = state
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
        .expect_err("OAuth accounts must reject standalone search overrides");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(err.1.contains("only supported for API key accounts"));

    let err = state
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
        .expect_err("OAuth accounts must reject clearing standalone search overrides");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);

    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load OAuth account after rejected override")
        .expect("OAuth account exists after rejected override");
    assert_eq!(row.policy_standalone_search_capability_override, None);
}

#[tokio::test]
pub(crate) async fn image_intent_explicit_unsupported_failure_learns_unsupported_capability() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Image Failure Learns Unsupported").await;

    record_pool_route_http_failure_for_endpoint_with_image_intent(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        false,
        Some("sticky-image-unsupported"),
        StatusCode::BAD_REQUEST,
        "pool upstream responded with 400: unsupported tool: image_generation is not supported by this account",
        Some("invk_image_unsupported"),
        "/v1/responses",
        ImageIntent::Yes,
    )
    .await
    .expect("record explicit unsupported image failure");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after unsupported image failure")
        .expect("row exists after unsupported image failure");
    assert_eq!(row.response_endpoint_capability.as_deref(), Some("unknown"));
    assert_eq!(
        row.response_image_tool_capability.as_deref(),
        Some("unsupported")
    );

    let direct_account_id =
        insert_oauth_account(&pool, "Direct Image Failure Learns Unsupported").await;
    record_pool_route_http_failure_for_endpoint_with_image_intent(
        &pool,
        direct_account_id,
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        false,
        Some("sticky-direct-image-unsupported"),
        StatusCode::BAD_REQUEST,
        "pool upstream responded with 400: No available channel for model gpt-image-1 under group default",
        Some("invk_direct_image_unsupported"),
        "/v1/images/generations",
        ImageIntent::DirectImage,
    )
    .await
    .expect("record explicit unsupported direct image failure");

    let direct_row = load_upstream_account_row(&pool, direct_account_id)
        .await
        .expect("load row after unsupported direct image failure")
        .expect("row exists after unsupported direct image failure");
    assert_eq!(
        direct_row.image_endpoint_capability.as_deref(),
        Some("unsupported")
    );
}

#[tokio::test]
pub(crate) async fn stale_capability_observations_cannot_overwrite_newer_results() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Fenced Capability Observations").await;
    let newer = "2026-09-13T10:00:01.000Z";
    let older = "2026-09-13T10:00:00.000Z";

    record_capability_observation_with_observed_at(
        &pool,
        account_id,
        UpstreamCapabilityAxis::ResponseEndpoint,
        CapabilitySupport::Supported,
        Some("newer success"),
        Some(newer),
    )
    .await
    .expect("record newer capability observation");
    record_capability_observation_with_observed_at(
        &pool,
        account_id,
        UpstreamCapabilityAxis::ResponseEndpoint,
        CapabilitySupport::Unsupported,
        Some("older failure"),
        Some(older),
    )
    .await
    .expect("record stale capability observation");

    record_compact_support_observation_with_observed_at(
        &pool,
        account_id,
        COMPACT_SUPPORT_STATUS_SUPPORTED,
        Some("newer success"),
        Some(newer),
    )
    .await
    .expect("record newer compact observation");
    record_compact_support_observation_with_observed_at(
        &pool,
        account_id,
        COMPACT_SUPPORT_STATUS_UNSUPPORTED,
        Some("older failure"),
        Some(older),
    )
    .await
    .expect("record stale compact observation");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load fenced observation row")
        .expect("fenced observation row exists");
    assert_eq!(
        row.response_endpoint_capability.as_deref(),
        Some(CapabilitySupport::Supported.as_str())
    );
    assert_eq!(
        row.response_endpoint_capability_observed_at.as_deref(),
        Some(newer)
    );
    assert_eq!(
        row.compact_support_status.as_deref(),
        Some(COMPACT_SUPPORT_STATUS_SUPPORTED)
    );
    assert_eq!(row.compact_support_observed_at.as_deref(), Some(newer));
}

#[tokio::test]
pub(crate) async fn image_intent_validation_failure_does_not_learn_unsupported_capability() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Image Validation Failure Keeps Unknown").await;

    record_pool_route_http_failure_for_endpoint_with_image_intent(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        false,
        Some("sticky-image-invalid-payload"),
        StatusCode::BAD_REQUEST,
        "pool upstream responded with 400: invalid image size: width must be divisible by 64",
        Some("invk_image_invalid_payload"),
        "/v1/responses",
        ImageIntent::Yes,
    )
    .await
    .expect("record image validation failure");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after image validation failure")
        .expect("row exists after image validation failure");
    assert_eq!(row.response_endpoint_capability.as_deref(), Some("unknown"));
    assert_eq!(
        row.response_image_tool_capability.as_deref(),
        Some("unknown")
    );
}

#[tokio::test]
pub(crate) async fn mark_account_sync_success_preserves_route_cooldown_state() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Cooldown OAuth").await;
    seed_route_cooldown(
        &pool,
        account_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        300,
    )
    .await;

    let before = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row before sync")
        .expect("row exists before sync");
    mark_account_sync_success(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MANUAL,
        SyncSuccessRouteState::PreserveFailureState,
    )
    .await
    .expect("mark sync success");
    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after sync")
        .expect("row exists after sync");

    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_synced_at.is_some());
    assert!(after.last_successful_sync_at.is_some());
    assert_eq!(after.last_route_failure_at, before.last_route_failure_at);
    assert_eq!(
        after.last_route_failure_kind,
        before.last_route_failure_kind
    );
    assert_eq!(after.cooldown_until, before.cooldown_until);
    assert_eq!(
        after.consecutive_route_failures,
        before.consecutive_route_failures
    );
}

#[tokio::test]
pub(crate) async fn mark_account_sync_success_clears_hard_unavailable_state_when_requested() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Recovered OAuth").await;
    seed_hard_unavailable_route_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    mark_account_sync_success(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MANUAL,
        SyncSuccessRouteState::ClearFailureState,
    )
    .await
    .expect("mark sync success");

    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after sync success")
        .expect("row exists after sync success");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_error.is_none());
    assert!(after.last_route_failure_kind.is_none());
    assert!(after.cooldown_until.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED)
    );
}

#[tokio::test]
pub(crate) async fn sync_api_key_account_preserves_route_cooldown_state() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Cooldown API Key").await;
    seed_route_cooldown(
        &pool,
        account_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        300,
    )
    .await;
    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load api key row")
        .expect("api key row exists");

    sync_api_key_account(&pool, &row, SyncCause::Manual)
        .await
        .expect("sync api key account");
    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after api key sync")
        .expect("row exists after api key sync");

    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    );
    assert!(after.cooldown_until.is_some());
    assert_eq!(after.consecutive_route_failures, 1);
}

#[tokio::test]
pub(crate) async fn sync_api_key_account_keeps_hard_unavailable_accounts_blocked() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Blocked API Key").await;
    seed_hard_unavailable_route_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load api key row")
        .expect("api key row exists");

    sync_api_key_account(&pool, &row, SyncCause::Manual)
        .await
        .expect("sync api key account");
    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after api key sync")
        .expect("row exists after api key sync");

    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert!(after.last_synced_at.is_some());
    assert!(after.last_successful_sync_at.is_none());
    assert_eq!(after.last_error.as_deref(), Some("seed hard unavailable"));
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_RECOVERY_BLOCKED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED)
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
}

#[tokio::test]
pub(crate) async fn sync_scope_reuses_live_reserved_node_for_same_account_before_shared_group_probe()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Reserved OAuth",
        "reserved@example.com",
        "org_reserved",
        "user_reserved",
    )
    .await;

    set_test_account_group_name(&state.pool, account_id, Some("node-shunt-sync-reserved")).await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-sync-reserved",
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
    .expect("save reserved node shunt sync metadata");
    drop(conn);

    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned")
        .insert(
            "test-node-shunt-sync-reservation".to_string(),
            PoolRoutingReservation {
                account_id,
                model: None,
                proxy_key: Some(FORWARD_PROXY_DIRECT_KEY.to_string()),
                created_at: Instant::now(),
            },
        );

    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load reserved account")
        .expect("reserved account exists");
    let scope = resolve_account_forward_proxy_scope_for_sync(state.as_ref(), &row, None)
        .await
        .expect("sync scope should reuse same-account live reservation");

    let ForwardProxyRouteScope::PinnedProxyKey(proxy_key) = scope else {
        panic!("expected sync scope to pin the live reserved node");
    };
    assert_eq!(proxy_key, FORWARD_PROXY_DIRECT_KEY);
}

#[tokio::test]
pub(crate) async fn oauth_sync_refresh_due_reuses_sync_only_scope_for_token_refresh() {
    let (proxy_url, usage_requests, token_requests, server) =
        spawn_proxy_only_oauth_sync_server().await;
    let state = test_app_state_with_usage_and_oauth_base(
        "http://unreachable.invalid/backend-api",
        "http://unreachable.invalid",
    )
    .await;
    let account_id = seed_refresh_due_account(&state, proxy_url).await;
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load refresh-due account")
        .expect("refresh-due account exists");
    sync_oauth_account(state.as_ref(), &row, SyncCause::Manual)
        .await
        .expect("refresh-due sync should reuse the sync-only scoped node for refresh");

    assert_successful_oauth_sync(
        state.as_ref(),
        account_id,
        &usage_requests,
        &token_requests,
        "proxy-refreshed-access-token",
        "proxy-refreshed-refresh-token",
    )
    .await;

    server.abort();
}

async fn seed_refresh_due_account(state: &Arc<AppState>, proxy_url: String) -> i64 {
    let secondary_proxy_key = {
        let mut manager = state.forward_proxy.lock().await;
        let settings = ForwardProxySettings {
            proxy_urls: vec![proxy_url],
            ..Default::default()
        };
        manager.apply_settings(settings);
        manager.bound_group_runtime.insert(
            "node-shunt-refresh".to_string(),
            crate::forward_proxy::BoundForwardProxyGroupState {
                current_binding_key: Some(FORWARD_PROXY_DIRECT_KEY.to_string()),
                consecutive_network_failures: 0,
            },
        );
        manager
            .binding_nodes()
            .into_iter()
            .find(|node| node.key != FORWARD_PROXY_DIRECT_KEY)
            .map(|node| node.key)
            .expect("secondary proxy binding key")
    };
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Refresh Due Scoped OAuth",
        "proxy-refresh@example.com",
        "org_proxy_refresh",
        "user_proxy_refresh",
    )
    .await;

    set_test_account_group_name(&state.pool, account_id, Some("node-shunt-refresh")).await;
    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-refresh",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: vec![
                FORWARD_PROXY_DIRECT_KEY.to_string(),
                secondary_proxy_key.clone(),
            ],
            node_shunt_enabled: true,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save refresh-due node shunt metadata");
    drop(conn);

    state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned")
        .insert(
            "test-node-shunt-refresh-reservation".to_string(),
            PoolRoutingReservation {
                account_id,
                model: None,
                proxy_key: Some(secondary_proxy_key),
                created_at: Instant::now(),
            },
        );
    set_test_account_token_expires_at(
        &state.pool,
        account_id,
        &format_utc_iso(Utc::now() - ChronoDuration::minutes(5)),
    )
    .await;
    account_id
}

#[tokio::test]
pub(crate) async fn sync_scope_falls_back_to_shared_bound_group_when_exclusive_slot_is_full() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let (occupying_account_id, queued_account_id) = seed_manual_node_shunt_accounts(&state).await;

    seed_hard_unavailable_route_failure(
        &state.pool,
        queued_account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    assert_node_shunt_unassigned(state.as_ref(), occupying_account_id, queued_account_id).await;

    let row = load_upstream_account_row(&state.pool, queued_account_id)
        .await
        .expect("load queued account")
        .expect("queued account exists");
    let scope = resolve_account_forward_proxy_scope_for_sync(state.as_ref(), &row, None)
        .await
        .expect("sync scope should fall back to shared bound-group probe");

    let ForwardProxyRouteScope::BoundGroup {
        group_name,
        bound_proxy_keys,
    } = scope
    else {
        panic!("expected sync scope to probe the bound group without claiming an exclusive slot");
    };
    assert_eq!(group_name, "node-shunt-sync");
    assert_eq!(bound_proxy_keys, test_required_group_bound_proxy_keys());
}

#[tokio::test]
pub(crate) async fn manual_sync_allows_group_node_shunt_unassigned_account_to_probe_bound_node() {
    let (base_url, server) = spawn_usage_snapshot_server(
        StatusCode::OK,
        json!({
            "planType": "team",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 42,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                }
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let (occupying_account_id, queued_account_id) = seed_manual_node_shunt_accounts(&state).await;

    seed_hard_unavailable_route_failure(
        &state.pool,
        queued_account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    assert_node_shunt_unassigned(state.as_ref(), occupying_account_id, queued_account_id).await;

    let detail = state
        .upstream_accounts
        .account_ops
        .run_manual_sync(state.clone(), queued_account_id)
        .await
        .expect("queued account manual sync should fall back to the shared bound node");

    let after = load_upstream_account_row(&state.pool, queued_account_id)
        .await
        .expect("load queued account after sync")
        .expect("queued account still exists");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_error.is_none());
    assert!(after.last_route_failure_kind.is_none());
    assert!(after.last_successful_sync_at.is_some());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED)
    );
    assert_eq!(detail.summary.id, queued_account_id);
    assert_eq!(
        detail.summary.routing_block_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_GROUP_NODE_SHUNT_UNASSIGNED),
    );
    assert_eq!(
        detail.summary.routing_block_reason_message.as_deref(),
        Some(group_node_shunt_unassigned_error_message()),
    );

    server.abort();
}
