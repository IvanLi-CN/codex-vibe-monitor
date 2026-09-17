use super::*;
use serde_json::json;

#[tokio::test]
pub(crate) async fn resolver_ignores_stale_sticky_routes_when_applying_concurrency_limit() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let limited_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Limited Account",
        "limited-stale@example.com",
        "org_limited_stale",
        "user_limited_stale",
    )
    .await;
    let fallback_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Fallback Account",
        "fallback-stale@example.com",
        "org_fallback_stale",
        "user_fallback_stale",
    )
    .await;

    sqlx::query("UPDATE pool_upstream_accounts SET group_name = ?2 WHERE id = ?1")
        .bind(limited_account_id)
        .bind("limited-stale")
        .execute(&state.pool)
        .await
        .expect("assign limited stale group");

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "limited-stale",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: test_required_group_bound_proxy_keys(),
            node_shunt_enabled: false,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 1,
        },
    )
    .await
    .expect("save limited stale group metadata");
    drop(conn);

    let stale_seen_at =
        format_utc_iso(Utc::now() - ChronoDuration::minutes(5) - ChronoDuration::seconds(1));
    upsert_sticky_route(
        &state.pool,
        "load-seed-stale",
        limited_account_id,
        &stale_seen_at,
    )
    .await
    .expect("seed stale sticky route");

    let resolution =
        resolve_pool_account_for_request(&state, None, &[], &std::collections::HashSet::new())
            .await
            .expect("resolve pool account");

    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected limited account to remain selectable");
    };
    assert_eq!(account.account_id, limited_account_id);
    assert_ne!(account.account_id, fallback_account_id);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::FreshAssignment
    );
}

#[tokio::test]
pub(crate) async fn resolver_allows_sticky_reuse_even_when_concurrency_limit_is_reached() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let limited_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Sticky Limited",
        "sticky-limited@example.com",
        "org_sticky_limited",
        "user_sticky_limited",
    )
    .await;

    sqlx::query("UPDATE pool_upstream_accounts SET group_name = ?2 WHERE id = ?1")
        .bind(limited_account_id)
        .bind("sticky-group")
        .execute(&state.pool)
        .await
        .expect("assign sticky group");

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "sticky-group",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: test_required_group_bound_proxy_keys(),
            node_shunt_enabled: false,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 1,
        },
    )
    .await
    .expect("save sticky group metadata");
    drop(conn);

    let now_iso = format_utc_iso(Utc::now());
    upsert_sticky_route(&state.pool, "sticky-reuse", limited_account_id, &now_iso)
        .await
        .expect("seed sticky route");

    let resolution = resolve_pool_account_for_request(
        &state,
        Some("sticky-reuse"),
        &[],
        &std::collections::HashSet::new(),
    )
    .await
    .expect("resolve sticky reuse");

    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected sticky reuse to resolve the existing account");
    };
    assert_eq!(account.account_id, limited_account_id);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::StickyReuse
    );
}

#[tokio::test]
pub(crate) async fn resolver_preserves_fallback_sticky_without_model() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let sticky_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Sticky Fallback Priority",
        "sk-sticky-priority-fallback",
        Some("sticky-priority"),
        Some("https://sticky-priority-fallback.example.com/backend-api/codex"),
    )
    .await;
    let primary_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Primary Replacement Candidate",
        "sk-sticky-priority-primary",
        Some("sticky-priority"),
        Some("https://sticky-priority-primary.example.com/backend-api/codex"),
    )
    .await;

    let mut fallback_rule = test_tag_routing_rule();
    fallback_rule.priority_tier = TagPriorityTier::Fallback;
    let fallback_tag = insert_test_tag(&state.pool, "sticky-fallback-priority", &fallback_rule)
        .await
        .expect("insert sticky fallback tag");
    let mut primary_rule = test_tag_routing_rule();
    primary_rule.priority_tier = TagPriorityTier::Primary;
    let primary_tag = insert_test_tag(&state.pool, "sticky-primary-priority", &primary_rule)
        .await
        .expect("insert sticky primary tag");
    sync_account_tag_links(&state.pool, sticky_account_id, &[fallback_tag.summary.id])
        .await
        .expect("attach sticky fallback tag");
    sync_account_tag_links(&state.pool, primary_account_id, &[primary_tag.summary.id])
        .await
        .expect("attach primary tag");

    let now_iso = format_utc_iso(Utc::now());
    upsert_sticky_route(
        &state.pool,
        "sticky-priority-reuse",
        sticky_account_id,
        &now_iso,
    )
    .await
    .expect("seed sticky route");
    insert_limit_sample_with_usage(
        &state.pool,
        sticky_account_id,
        &now_iso,
        Some(5.0),
        Some(1.0),
    )
    .await;
    insert_limit_sample_with_usage(
        &state.pool,
        primary_account_id,
        &now_iso,
        Some(1.0),
        Some(1.0),
    )
    .await;

    let resolution = resolve_pool_account_for_request(
        &state,
        Some("sticky-priority-reuse"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve sticky reuse with higher priority candidate");

    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected fallback sticky route to remain on the source without a model");
    };
    assert_eq!(account.account_id, sticky_account_id);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::StickyReuse
    );
}

#[tokio::test]
pub(crate) async fn maintenance_pass_skips_secondary_overflow_accounts_until_secondary_interval() {
    let (usage_base_url, requests, server) = spawn_secondary_usage_server().await;

    let state = test_app_state_with_usage_base(&usage_base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let priority_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Priority OAuth",
        "priority@example.com",
        "org_priority",
        "user_priority",
    )
    .await;
    let secondary_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Secondary OAuth",
        "secondary@example.com",
        "org_secondary",
        "user_secondary",
    )
    .await;

    save_pool_routing_maintenance_settings(
        &state.pool,
        PoolRoutingMaintenanceSettings {
            primary_sync_interval_secs: 300,
            secondary_sync_interval_secs: 1800,
            priority_available_account_cap: 1,
        },
    )
    .await
    .expect("save maintenance settings");
    insert_limit_sample_with_usage(
        &state.pool,
        priority_account_id,
        "2026-03-23T11:00:00Z",
        Some(12.0),
        Some(10.0),
    )
    .await;
    insert_limit_sample_with_usage(
        &state.pool,
        secondary_account_id,
        "2026-03-23T11:00:00Z",
        Some(14.0),
        Some(20.0),
    )
    .await;
    let last_synced_at = format_utc_iso(Utc::now() - ChronoDuration::minutes(10));
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET last_synced_at = ?3,
                last_successful_sync_at = ?3
            WHERE id IN (?1, ?2)
            "#,
    )
    .bind(priority_account_id)
    .bind(secondary_account_id)
    .bind(&last_synced_at)
    .execute(&state.pool)
    .await
    .expect("seed sync times");

    run_upstream_account_maintenance_once(state.clone())
        .await
        .expect("run maintenance pass");
    timeout(Duration::from_secs(8), async {
        while requests.load(Ordering::SeqCst) < 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("priority maintenance request should finish");
    tokio::time::sleep(Duration::from_millis(150)).await;

    assert_eq!(
        requests.load(Ordering::SeqCst),
        1,
        "overflow secondary account should not sync on the primary interval"
    );
    server.abort();
}

async fn spawn_secondary_usage_server() -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    async fn handler(State(requests): State<Arc<AtomicUsize>>) -> (StatusCode, String) {
        requests.fetch_add(1, Ordering::SeqCst);
        (
            StatusCode::OK,
            json!({
                "planType": "team",
                "rateLimit": {
                    "primaryWindow": {
                        "usedPercent": 8,
                        "windowDurationMins": 300,
                        "resetsAt": 1771322400
                    },
                    "secondaryWindow": {
                        "usedPercent": 8,
                        "windowDurationMins": 10080,
                        "resetsAt": 1771927200
                    }
                }
            })
            .to_string(),
        )
    }

    let requests = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/backend-api/wham/usage", get(handler))
        .with_state(requests.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind usage server");
    let addr = listener.local_addr().expect("usage server addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve usage server");
    });

    (format!("http://{addr}/backend-api"), requests, server)
}

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

    let row = load_upstream_account_row(&pool, account_id)
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
        load_sticky_route(&pool, "sticky-degraded-cooldown")
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

    assert_transport_failure_cooldown_cap(&pool, account_id).await;
}

async fn assert_transport_failure_cooldown_cap(pool: &SqlitePool, account_id: i64) {
    let (raw_failure_at, raw_cooldown_until): (String, String) = sqlx::query_as(
        "SELECT last_route_failure_at, cooldown_until FROM pool_upstream_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
    .expect("load raw millisecond cooldown values");
    assert!(raw_failure_at.contains('.'));
    assert!(raw_cooldown_until.contains('.'));

    let row = load_upstream_account_row(pool, account_id)
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

#[test]
pub(crate) fn classify_pool_account_http_failure_treats_usage_limit_reached_as_quota_exhausted() {
    let classification = classify_pool_account_http_failure(
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        StatusCode::TOO_MANY_REQUESTS,
        "pool upstream responded with 429: The usage limit has been reached",
    );

    assert_eq!(
        classification.disposition,
        UpstreamAccountFailureDisposition::RateLimited
    );
    assert_eq!(
        classification.reason_code,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED
    );
    assert_eq!(
        classification.failure_kind,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED
    );
}

#[tokio::test]
pub(crate) async fn quota_exhausted_oauth_summary_and_detail_export_as_rate_limited() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Quota Exhausted OAuth").await;

    record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            false,
            Some("sticky-quota-exhausted"),
            StatusCode::TOO_MANY_REQUESTS,
            "oauth_upstream_rejected_request: pool upstream responded with 429: The usage limit has been reached",
            Some("invk_quota_exhausted"),
        )
        .await
        .expect("record wrapped 429 route failure");

    let route_failure_row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load route failure row")
        .expect("route failure row exists");
    record_account_sync_recovery_blocked(
        &pool,
        account_id,
        &route_failure_row.status,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
        &route_failure_row.status,
        UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED,
        "latest usage snapshot still shows an exhausted upstream usage limit window",
        route_failure_row.last_error.as_deref(),
        route_failure_row.last_route_failure_kind.as_deref(),
    )
    .await
    .expect("record blocked sync recovery");

    sqlx::query(
        r#"
            INSERT INTO pool_upstream_account_limit_samples (
                account_id, captured_at, limit_id, limit_name, plan_type,
                primary_used_percent, primary_window_minutes, primary_resets_at,
                secondary_used_percent, secondary_window_minutes, secondary_resets_at,
                credits_has_credits, credits_unlimited, credits_balance
            ) VALUES (
                ?1, ?2, NULL, NULL, 'team',
                100.0, 300, ?3,
                64.0, 10080, ?4,
                1, 0, '0.00'
            )
            "#,
    )
    .bind(account_id)
    .bind("2026-03-24T18:00:27Z")
    .bind("2026-03-30T16:06:33Z")
    .bind("2026-04-01T00:00:00Z")
    .execute(&pool)
    .await
    .expect("insert exhausted usage sample");

    assert_quota_exhausted_summary_and_detail(&pool, account_id).await;
}

async fn assert_quota_exhausted_summary_and_detail(pool: &SqlitePool, account_id: i64) {
    let row = load_upstream_account_row(pool, account_id)
        .await
        .expect("load updated row")
        .expect("updated row exists");
    let latest = load_latest_usage_sample(pool, account_id)
        .await
        .expect("load latest usage sample");
    let summary = build_summary_from_row(
        &row,
        latest.as_ref(),
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
        summary.last_error.as_deref(),
        Some(
            "oauth_upstream_rejected_request: pool upstream responded with 429: The usage limit has been reached"
        )
    );
    assert_eq!(
        summary.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)
    );
    assert_eq!(
        summary
            .primary_window
            .as_ref()
            .map(|window| window.used_percent),
        Some(100.0)
    );
    assert_eq!(
        summary
            .primary_window
            .as_ref()
            .and_then(|window| window.resets_at.as_deref()),
        Some("2026-03-30T16:06:33Z")
    );

    let detail = load_upstream_account_detail(pool, account_id)
        .await
        .expect("load detail export")
        .expect("detail export exists");
    assert_eq!(
        detail.summary.display_status,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE
    );
    assert_eq!(
        detail.summary.health_status,
        UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL
    );
    assert_eq!(
        detail.summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(
        detail.summary.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)
    );
    assert_eq!(
        detail
            .recent_actions
            .first()
            .map(|event| event.action.as_str()),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_RECOVERY_BLOCKED)
    );
    assert_eq!(
        detail
            .recent_actions
            .first()
            .and_then(|event| event.reason_code.as_deref()),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)
    );
    assert_eq!(
        detail
            .recent_actions
            .first()
            .and_then(|event| event.failure_kind.as_deref()),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
}
