#[tokio::test]
pub(crate) async fn account_actors_are_released_after_idle_commands_finish() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Actor cleanup").await;

    assert_eq!(state.upstream_accounts.account_ops.actor_count(), 0);

    state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            account_id,
            UpdateUpstreamAccountRequest {
                display_name: None,
                email: OptionalField::Missing,
                group_name: None,
                group_bound_proxy_keys: None,
                group_node_shunt_enabled: None,
                group_single_account_rotation_enabled: None,
                note: Some("released".to_string()),
                group_note: None,
                concurrency_limit: None,
                upstream_base_url: OptionalField::Missing,
                bound_proxy_keys: OptionalField::Missing,
                enabled: None,
                is_mother: None,
                api_key: None,
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: None,
                routing_rule: None,
                ..UpdateUpstreamAccountRequest::default()
            },
        )
        .await
        .expect("update account");
    assert_eq!(state.upstream_accounts.account_ops.actor_count(), 0);

    state
        .upstream_accounts
        .account_ops
        .run_delete_account(state.clone(), account_id)
        .await
        .expect("delete account");
    assert_eq!(state.upstream_accounts.account_ops.actor_count(), 0);
}

#[tokio::test]
pub(crate) async fn record_pool_route_http_failure_keeps_missing_scope_oauth_as_error() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Scope OAuth").await;

    record_pool_route_http_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        false,
        Some("sticky-scope"),
        StatusCode::UNAUTHORIZED,
        "pool upstream responded with 401: Missing scopes: api.responses.write",
        None,
    )
    .await
    .expect("record route failure");

    let status: String =
        sqlx::query_scalar("SELECT status FROM pool_upstream_accounts WHERE id = ?1")
            .bind(account_id)
            .fetch_one(&pool)
            .await
            .expect("load account status");
    assert_eq!(status, UPSTREAM_ACCOUNT_STATUS_ERROR);
}

#[tokio::test]
pub(crate) async fn record_pool_route_http_failure_emits_suppressed_event_when_reason_toggle_disabled()
 {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Suppressed Scope OAuth").await;
    upsert_sticky_route(
        &pool,
        "sticky-suppressed-scope",
        account_id,
        &format_utc_iso(Utc::now()),
    )
    .await
    .expect("seed suppressed sticky route");
    sqlx::query(
            "UPDATE pool_upstream_accounts SET policy_status_change_upstream_http_401 = 0 WHERE id = ?1",
        )
        .bind(account_id)
        .execute(&pool)
        .await
        .expect("disable 401 status change toggle");

    record_pool_route_http_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        false,
        Some("sticky-suppressed-scope"),
        StatusCode::UNAUTHORIZED,
        "pool upstream responded with 401: Missing scopes: api.responses.write",
        Some("invk_suppressed_scope"),
    )
    .await
    .expect("record suppressed route failure");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load suppressed row")
        .expect("suppressed row exists");
    assert_eq!(row.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(row.last_error, None);
    assert_eq!(row.last_action, None);
    assert_eq!(row.last_route_failure_kind, None);
    assert_eq!(row.cooldown_until, None);
    assert_eq!(row.consecutive_route_failures, 0);
    assert_eq!(
        load_sticky_route(&pool, "sticky-suppressed-scope")
            .await
            .expect("load suppressed sticky route")
            .map(|route| route.account_id),
        Some(account_id),
    );

    let detail = load_upstream_account_detail(&pool, account_id)
        .await
        .expect("load suppressed detail")
        .expect("suppressed detail exists");
    let event = detail
        .recent_actions
        .first()
        .expect("suppressed event should be recorded");
    assert_eq!(
        event.action,
        UPSTREAM_ACCOUNT_ACTION_STATUS_CHANGE_SUPPRESSED
    );
    assert_eq!(event.source, UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL);
    assert_eq!(
        event.reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_401)
    );
    assert_eq!(event.http_status, Some(401));
    assert_eq!(
        event.failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_AUTH)
    );
    assert!(
        event
            .reason_message
            .as_deref()
            .is_some_and(|value| value.contains("Missing scopes: api.responses.write"))
    );
}

#[tokio::test]
pub(crate) async fn record_pool_route_http_failure_preserves_sticky_route_when_single_account_rotation_429_is_suppressed()
 {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Suppressed Single Rotation 429").await;
    upsert_sticky_route(
        &pool,
        "sticky-suppressed-429",
        account_id,
        &format_utc_iso(Utc::now()),
    )
    .await
    .expect("seed suppressed 429 sticky route");
    sqlx::query(
            "UPDATE pool_upstream_accounts SET policy_status_change_upstream_http_429_rate_limit = 0 WHERE id = ?1",
        )
        .bind(account_id)
        .execute(&pool)
        .await
        .expect("disable 429 rate-limit status change toggle");

    record_pool_route_http_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        true,
        Some("sticky-suppressed-429"),
        StatusCode::TOO_MANY_REQUESTS,
        "pool upstream responded with 429: too many requests",
        Some("invk_suppressed_429"),
    )
    .await
    .expect("record suppressed single-account rotation 429");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load suppressed 429 row")
        .expect("suppressed 429 row exists");
    assert_eq!(row.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(row.last_error, None);
    assert_eq!(row.last_action, None);
    assert_eq!(row.last_route_failure_kind, None);
    assert_eq!(row.cooldown_until, None);
    assert_eq!(row.consecutive_route_failures, 0);
    assert_eq!(
        load_sticky_route(&pool, "sticky-suppressed-429")
            .await
            .expect("load suppressed 429 sticky route")
            .map(|route| route.account_id),
        Some(account_id),
    );

    let detail = load_upstream_account_detail(&pool, account_id)
        .await
        .expect("load suppressed 429 detail")
        .expect("suppressed 429 detail exists");
    let event = detail
        .recent_actions
        .first()
        .expect("suppressed 429 event should be recorded");
    assert_eq!(
        event.action,
        UPSTREAM_ACCOUNT_ACTION_STATUS_CHANGE_SUPPRESSED
    );
    assert_eq!(event.source, UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL);
    assert_eq!(
        event.reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT)
    );
    assert_eq!(event.http_status, Some(429));
    assert_eq!(
        event.failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    );
}

#[tokio::test]
pub(crate) async fn automatic_single_rotation_429_clear_broadcasts_conversation_change() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_oauth_account(&state.pool, "Single Rotation 429").await;
    let prompt_cache_key = "pck-single-rotation";
    let mut broadcast_receiver = state.broadcaster.subscribe();
    upsert_sticky_route(
        &state.pool,
        prompt_cache_key,
        account_id,
        &format_utc_iso(Utc::now()),
    )
    .await
    .expect("seed sticky route");
    let generation_before = load_sticky_affinity_generation(&state.pool, prompt_cache_key)
        .await
        .expect("load sticky generation before 429");

    record_pool_route_http_failure_for_endpoint_with_image_intent_and_prompt_cache_key_for_attempt_and_broadcast(
        state.as_ref(),
        account_id,
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        true,
        Some(prompt_cache_key),
        StatusCode::TOO_MANY_REQUESTS,
        "pool upstream responded with 429: too many requests",
        None,
        "/v1/responses",
        ImageIntent::Unknown,
        None,
        None,
        Some(prompt_cache_key),
    )
    .await
    .expect("record single-account rotation 429");

    assert!(
        load_sticky_route(&state.pool, prompt_cache_key)
            .await
            .expect("load sticky route")
            .is_none(),
        "the conversation should be unbound so the next attempt can pick the next candidate",
    );
    assert_eq!(
        load_sticky_affinity_generation(&state.pool, prompt_cache_key)
            .await
            .expect("load sticky generation after 429"),
        generation_before + 1,
        "automatic 429 clear must fence stale completions",
    );
    assert!(matches!(
        broadcast_receiver.recv().await.expect("conversation change broadcast"),
        BroadcastPayload::PromptCacheConversationChanged { prompt_cache_key: key }
            if key == prompt_cache_key
    ));
}

#[tokio::test]
pub(crate) async fn stale_failure_does_not_clear_rebound_sticky_target_for_same_account() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Rebound Sticky Account").await;
    let sticky_key = "sticky-stale-failure-rebound";
    let now_iso = format_utc_iso(Utc::now());

    upsert_sticky_route(&pool, sticky_key, account_id, &now_iso)
        .await
        .expect("seed original sticky route");
    let stale_generation = load_sticky_affinity_generation(&pool, sticky_key)
        .await
        .expect("load original sticky generation");
    bump_sticky_affinity_generation(&pool, sticky_key, &now_iso)
        .await
        .expect("simulate a newer rebound generation");

    assert!(
        !delete_sticky_route_if_matches(
            &pool,
            sticky_key,
            account_id,
            Some(stale_generation),
            &now_iso,
        )
        .await
        .expect("stale clear should be suppressed"),
        "an old failure must not clear the same account after a newer sticky generation",
    );
    assert_eq!(
        load_sticky_route(&pool, sticky_key)
            .await
            .expect("load rebound sticky route")
            .map(|route| route.account_id),
        Some(account_id),
    );
}

#[tokio::test]
pub(crate) async fn record_pool_route_http_failure_preserves_sticky_route_when_single_account_rotation_is_off()
 {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Legacy 429 Sticky").await;
    upsert_sticky_route(
        &pool,
        "sticky-legacy-429",
        account_id,
        &format_utc_iso(Utc::now()),
    )
    .await
    .expect("seed sticky route");

    record_pool_route_http_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        false,
        Some("sticky-legacy-429"),
        StatusCode::TOO_MANY_REQUESTS,
        "pool upstream responded with 429: too many requests",
        None,
    )
    .await
    .expect("record legacy 429");

    assert!(
        load_sticky_route(&pool, "sticky-legacy-429")
            .await
            .expect("load sticky route")
            .is_some(),
        "legacy groups keep existing sticky route behavior",
    );
}

#[tokio::test]
pub(crate) async fn record_pool_route_http_failure_does_not_learn_feature_level_model_error_as_system_deny()
 {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Feature Unsupported OAuth").await;

    record_pool_route_http_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        false,
        Some("sticky-feature-unsupported"),
        StatusCode::BAD_REQUEST,
        "unsupported_model: response_format is not supported for model gpt-4o",
        Some("invk_feature_unsupported"),
    )
    .await
    .expect("record feature-scoped bad request");

    let learned_tags = sqlx::query_scalar::<_, String>(
        r#"
            SELECT tag.system_key
            FROM pool_upstream_account_tags account_tag
            JOIN pool_tags tag ON tag.id = account_tag.tag_id
            WHERE account_tag.account_id = ?1
              AND tag.system_key IS NOT NULL
            ORDER BY tag.system_key ASC
            "#,
    )
    .bind(account_id)
    .fetch_all(&pool)
    .await
    .expect("load learned system tags");
    assert!(
        !learned_tags
            .iter()
            .any(|tag| tag == "unsupported_model:gpt-4o"),
        "feature-scoped bad request should not poison model availability: {learned_tags:?}",
    );
}

#[tokio::test]
pub(crate) async fn record_pool_route_http_failure_marks_explicit_invalidated_oauth_for_reauth() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Invalidated OAuth").await;

    record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            false,
            Some("sticky-invalidated"),
            StatusCode::FORBIDDEN,
            "pool upstream responded with 403: Authentication token has been invalidated, please sign in again",
            None,
        )
        .await
        .expect("record route failure");

    let status: String =
        sqlx::query_scalar("SELECT status FROM pool_upstream_accounts WHERE id = ?1")
            .bind(account_id)
            .fetch_one(&pool)
            .await
            .expect("load account status");
    assert_eq!(status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
}

#[tokio::test]
pub(crate) async fn record_pool_route_http_failure_keeps_bridge_exchange_oauth_as_error() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Bridge OAuth").await;

    record_pool_route_http_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        false,
        Some("sticky-bridge"),
        StatusCode::UNAUTHORIZED,
        "oauth bridge token exchange failed: oauth bridge responded with 502",
        None,
    )
    .await
    .expect("record route failure");

    let status: String =
        sqlx::query_scalar("SELECT status FROM pool_upstream_accounts WHERE id = ?1")
            .bind(account_id)
            .fetch_one(&pool)
            .await
            .expect("load account status");
    assert_eq!(status, UPSTREAM_ACCOUNT_STATUS_ERROR);
}

#[tokio::test]
pub(crate) async fn record_pool_route_http_failure_marks_402_as_hard_error_and_records_reason() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Plan Blocked Key").await;

    record_pool_route_http_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
        false,
        Some("sticky-402"),
        StatusCode::PAYMENT_REQUIRED,
        "pool upstream responded with 402: subscription required",
        Some("invk_402"),
    )
    .await
    .expect("record route failure");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load account row")
        .expect("account should exist");
    assert_eq!(row.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert_eq!(
        row.last_action_reason_code.as_deref(),
        Some("upstream_http_402")
    );
    assert_eq!(
        row.last_action_source.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL)
    );
    assert_eq!(row.last_action_http_status, Some(402));
    assert_eq!(
        row.last_route_failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_402)
    );
    assert!(row.cooldown_until.is_none());
}

#[tokio::test]
pub(crate) async fn route_triggered_402_summary_and_detail_export_as_upstream_rejected() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Workspace Blocked OAuth").await;

    record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            false,
            Some("sticky-402-workspace"),
            StatusCode::PAYMENT_REQUIRED,
            "initial usage snapshot attempt with configured user agent failed: usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}",
            Some("invk_workspace_402"),
        )
        .await
        .expect("record route-triggered 402 failure");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load route-triggered 402 row")
        .expect("route-triggered 402 row exists");
    let summary = build_summary_from_row(
        &row,
        None,
        row.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );

    assert_eq!(
        summary.display_status,
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
    );
    assert_eq!(
        summary.health_status,
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
    );
    assert_eq!(
        summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
    );
    assert_eq!(
        summary.last_action_reason_code.as_deref(),
        Some("upstream_http_402")
    );
    assert_eq!(
        summary.last_error.as_deref(),
        Some(
            "initial usage snapshot attempt with configured user agent failed: usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}"
        )
    );

    let detail = load_upstream_account_detail(&pool, account_id)
        .await
        .expect("load route-triggered 402 detail")
        .expect("route-triggered 402 detail exists");
    assert_eq!(
        detail.summary.display_status,
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
    );
    assert_eq!(
        detail.summary.health_status,
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
    );
    assert_eq!(
        detail.summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
    );
    assert_eq!(
        detail.summary.last_action_reason_code.as_deref(),
        Some("upstream_http_402")
    );
    assert_eq!(
        detail
            .recent_actions
            .first()
            .and_then(|event| event.reason_code.as_deref()),
        Some("upstream_http_402")
    );
    assert_eq!(
        detail
            .recent_actions
            .first()
            .and_then(|event| event.failure_kind.as_deref()),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_402)
    );
}

#[tokio::test]
pub(crate) async fn record_pool_route_http_failure_keeps_unattributed_api_key_quota_429_diagnostic_only()
 {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Quota Exhausted Key").await;
    upsert_sticky_route(
        &pool,
        "sticky-429-quota",
        account_id,
        &format_utc_iso(Utc::now()),
    )
    .await
    .expect("seed quota sticky route");

    record_pool_route_http_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
        false,
        Some("sticky-429-quota"),
        StatusCode::TOO_MANY_REQUESTS,
        "insufficient_quota: pool upstream responded with 429: weekly cap exhausted",
        Some("invk_quota_429"),
    )
    .await
    .expect("record route failure");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load account row")
        .expect("account should exist");
    assert_eq!(row.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(row.last_action.is_none());
    assert!(row.last_action_reason_code.is_none());
    assert!(row.last_action_http_status.is_none());
    assert!(row.last_route_failure_kind.is_none());
    assert!(row.cooldown_until.is_none());
    assert_eq!(row.consecutive_route_failures, 0);
    assert_eq!(
        load_sticky_route(&pool, "sticky-429-quota")
            .await
            .expect("load quota sticky route")
            .map(|route| route.account_id),
        Some(account_id),
    );
}

#[tokio::test]
pub(crate) async fn record_pool_route_http_failure_keeps_unattributed_api_key_plain_429_diagnostic_only()
 {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Degraded Plain 429").await;
    upsert_sticky_route(
        &pool,
        "sticky-degraded-first-hit",
        account_id,
        &format_utc_iso(Utc::now()),
    )
    .await
    .expect("seed sticky route");

    record_pool_route_http_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
        false,
        Some("sticky-degraded-first-hit"),
        StatusCode::TOO_MANY_REQUESTS,
        "pool upstream responded with 429: too many requests",
        Some("invk_degraded_first_hit"),
    )
    .await
    .expect("record first degraded 429 failure");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load degraded row")
        .expect("degraded row exists");
    assert_eq!(row.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(row.last_action.is_none());
    assert!(row.last_action_reason_code.is_none());
    assert!(row.last_route_failure_kind.is_none());
    assert!(row.cooldown_until.is_none());
    assert_eq!(row.consecutive_route_failures, 0);
    assert!(row.temporary_route_failure_streak_started_at.is_none());
    assert_eq!(
        load_sticky_route(&pool, "sticky-degraded-first-hit")
            .await
            .expect("load sticky route after degraded hit")
            .map(|route| route.account_id),
        Some(account_id)
    );

    let summary = build_summary_from_row(
        &row,
        None,
        row.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(summary.work_status, UPSTREAM_ACCOUNT_WORK_STATUS_IDLE);
}

#[tokio::test]
pub(crate) async fn record_pool_route_http_failure_keeps_unattributed_api_key_overload_diagnostic_only()
 {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Overloaded Key").await;

    record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
            false,
            Some("sticky-overloaded"),
            StatusCode::OK,
            "[upstream_response_failed] server_is_overloaded: Our servers are currently overloaded. Please try again later.",
            Some("invk_overloaded"),
        )
        .await
        .expect("record retryable overload route failure");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load overloaded row")
        .expect("overloaded row exists");
    assert_eq!(row.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(row.last_action.is_none());
    assert!(row.last_action_reason_code.is_none());
    assert!(row.last_action_http_status.is_none());
    assert!(row.last_route_failure_kind.is_none());
    assert!(row.cooldown_until.is_none());
    assert_eq!(row.consecutive_route_failures, 0);
    assert!(row.temporary_route_failure_streak_started_at.is_none());

    let summary = build_summary_from_row(
        &row,
        None,
        row.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(summary.work_status, UPSTREAM_ACCOUNT_WORK_STATUS_IDLE);
}

#[tokio::test]
pub(crate) async fn record_pool_route_transport_failure_starts_temporary_cooldown_after_streak_window_expires()
 {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Cooldown Escalation").await;
    upsert_sticky_route(
        &pool,
        "sticky-degraded-cooldown",
        account_id,
        &format_utc_iso(Utc::now()),
    )
    .await
    .expect("seed sticky route");

    record_pool_route_transport_failure(
        &pool,
        account_id,
        Some("sticky-degraded-cooldown"),
        "failed to contact upstream",
        Some("invk_transport_first"),
    )
    .await
    .expect("record first transport failure");

    let stale_started_at = format_utc_iso(
        Utc::now() - ChronoDuration::seconds(POOL_ROUTE_TEMPORARY_FAILURE_DEGRADED_WINDOW_SECS + 1),
    );
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET temporary_route_failure_streak_started_at = ?2
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind(&stale_started_at)
    .execute(&pool)
    .await
    .expect("stale degraded streak start");

    record_pool_route_transport_failure(
        &pool,
        account_id,
        Some("sticky-degraded-cooldown"),
        "failed to contact upstream again",
        Some("invk_transport_second"),
    )
    .await
    .expect("record second transport failure");

    assert_transport_failure_cooldown_escalation(&pool, account_id).await;
}

async fn assert_transport_failure_cooldown_escalation(pool: &SqlitePool, account_id: i64) {
    let row = load_upstream_account_row(pool, account_id)
        .await
        .expect("load escalated row")
        .expect("escalated row exists");
    assert_eq!(
        row.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_ROUTE_COOLDOWN_STARTED)
    );
    assert_eq!(
        row.last_route_failure_kind.as_deref(),
        Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM)
    );
    assert!(row.cooldown_until.is_some());
    assert_eq!(row.consecutive_route_failures, 2);
    assert_eq!(
        load_sticky_route(pool, "sticky-degraded-cooldown")
            .await
            .expect("load sticky route after cooldown escalation")
            .map(|route| route.account_id),
        Some(account_id)
    );

    let summary = build_summary_from_row(
        &row,
        None,
        row.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(summary.work_status, UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED);
}

#[tokio::test]
pub(crate) async fn route_failure_event_records_the_exact_upstream_attempt_id() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Attempt Event Link").await;

    record_pool_route_transport_failure_for_attempt(
        &pool,
        account_id,
        None,
        "failed to contact upstream",
        Some("invk_attempt_event_link"),
        Some(9_102),
    )
    .await
    .expect("record route failure with attempt id");

    let attempt_id = sqlx::query_scalar::<_, Option<i64>>(
        r#"
        SELECT attempt_id
        FROM pool_upstream_account_events
        WHERE account_id = ?1 AND invoke_id = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(account_id)
    .bind("invk_attempt_event_link")
    .fetch_one(&pool)
    .await
    .expect("load route failure event");

    assert_eq!(attempt_id, Some(9_102));
}

#[tokio::test]
pub(crate) async fn route_success_event_records_the_exact_upstream_attempt_id() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Attempt Success Event Link").await;

    record_pool_route_success_with_image_intent_for_attempt(
        &pool,
        account_id,
        Utc::now(),
        None,
        Some("invk_attempt_success_event_link"),
        ImageIntent::Unknown,
        Some(9_103),
    )
    .await
    .expect("record route success with attempt id");

    let attempt_id = sqlx::query_scalar::<_, Option<i64>>(
        r#"
        SELECT attempt_id
        FROM pool_upstream_account_events
        WHERE account_id = ?1 AND invoke_id = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(account_id)
    .bind("invk_attempt_success_event_link")
    .fetch_one(&pool)
    .await
    .expect("load route success event");

    assert_eq!(attempt_id, Some(9_103));
}

#[tokio::test]
pub(crate) async fn record_pool_route_transport_failure_caps_temporary_cooldown_at_sixty_seconds() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Cooldown Cap").await;
    let baseline_now = Utc::now();
    let baseline_now_iso = format_utc_iso(baseline_now);
    let stale_started_at = format_utc_iso(
        baseline_now
            - ChronoDuration::seconds(POOL_ROUTE_TEMPORARY_FAILURE_DEGRADED_WINDOW_SECS + 5),
    );
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET status = ?2,
                last_error = ?3,
                last_error_at = ?4,
                last_route_failure_at = ?4,
                last_route_failure_kind = ?5,
                cooldown_until = NULL,
                consecutive_route_failures = ?6,
                temporary_route_failure_streak_started_at = ?7,
                updated_at = ?4
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind("previous temporary failure")
    .bind(&baseline_now_iso)
    .bind(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM)
    .bind(7_i64)
    .bind(&stale_started_at)
    .execute(&pool)
    .await
    .expect("seed high temporary failure streak");

    record_pool_route_transport_failure(
        &pool,
        account_id,
        None,
        "failed to contact upstream again",
        Some("invk_transport_cap"),
    )
    .await
    .expect("record capped transport failure");

    let (raw_failure_at, raw_cooldown_until): (String, String) = sqlx::query_as(
        "SELECT last_route_failure_at, cooldown_until FROM pool_upstream_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(&pool)
    .await
    .expect("load raw millisecond cooldown values");
    assert!(raw_failure_at.contains('.'));
    assert!(raw_cooldown_until.contains('.'));

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load capped row")
        .expect("capped row exists");
    assert_eq!(
        row.last_route_failure_at.as_deref(),
        Some(raw_failure_at.as_str())
    );
    assert_eq!(
        row.cooldown_until.as_deref(),
        Some(raw_cooldown_until.as_str())
    );
    let cooldown_until = row
        .cooldown_until
        .as_deref()
        .and_then(parse_rfc3339_utc)
        .expect("cooldown should be set");
    assert!(
        row.cooldown_until
            .as_deref()
            .is_some_and(|value| value.contains('.')),
        "new account cooldowns should retain millisecond precision"
    );
    let route_failure_at = row
        .last_route_failure_at
        .as_deref()
        .and_then(parse_rfc3339_utc)
        .expect("route failure timestamp should be set");
    assert_eq!(parse_rfc3339_utc(&raw_cooldown_until), Some(cooldown_until));
    assert_eq!(
        cooldown_until - route_failure_at,
        ChronoDuration::seconds(POOL_ROUTE_TEMPORARY_FAILURE_COOLDOWN_MAX_SECS)
    );
    assert!(account_has_active_cooldown(
        row.cooldown_until.as_deref(),
        cooldown_until - ChronoDuration::milliseconds(1)
    ));
    assert!(!account_has_active_cooldown(
        row.cooldown_until.as_deref(),
        cooldown_until
    ));

    assert_legacy_second_precision_cooldown(&pool, account_id, cooldown_until).await;
}

async fn assert_legacy_second_precision_cooldown(
    pool: &SqlitePool,
    account_id: i64,
    cooldown_until: DateTime<Utc>,
) {
    let legacy_cooldown_until = format_utc_iso(cooldown_until + ChronoDuration::seconds(60));
    sqlx::query("UPDATE pool_upstream_accounts SET cooldown_until = ?2 WHERE id = ?1")
        .bind(account_id)
        .bind(&legacy_cooldown_until)
        .execute(pool)
        .await
        .expect("replace cooldown with a legacy second-precision value");
    let legacy_row = load_upstream_account_row(pool, account_id)
        .await
        .expect("reload legacy cooldown row")
        .expect("legacy cooldown row exists");
    let legacy_until = parse_rfc3339_utc(&legacy_cooldown_until).expect("parse legacy cooldown");
    assert!(account_has_active_cooldown(
        legacy_row.cooldown_until.as_deref(),
        legacy_until - ChronoDuration::milliseconds(1)
    ));
    assert!(!account_has_active_cooldown(
        legacy_row.cooldown_until.as_deref(),
        legacy_until
    ));
}

use super::*;
