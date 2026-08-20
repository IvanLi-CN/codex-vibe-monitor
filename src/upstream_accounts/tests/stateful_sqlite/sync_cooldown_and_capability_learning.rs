use super::*;
use serde_json::json;

#[test]
fn explicit_model_failure_recognizes_standard_not_found_and_rate_limit_shapes() {
    assert!(is_explicit_model_failure(
        StatusCode::NOT_FOUND,
        Some(r#"{"code":"model_not_found","message":"model gpt-5.5 does not exist"}"#),
    ));
    assert!(!is_explicit_model_failure_for_model(
        StatusCode::NOT_FOUND,
        Some("model gpt-5.4 does not exist"),
        Some("gpt-5.5"),
    ));
    assert!(is_explicit_model_failure_for_model(
        StatusCode::NOT_FOUND,
        Some("model gpt-5.5 does not exist"),
        Some("gpt-5.5"),
    ));
    assert!(is_explicit_model_failure(
        StatusCode::TOO_MANY_REQUESTS,
        Some("rate limit reached for gpt-5.5"),
    ));
    assert!(!is_explicit_model_failure(
        StatusCode::TOO_MANY_REQUESTS,
        Some("rate limit reached for this account"),
    ));
    assert!(!is_explicit_model_failure(
        StatusCode::TOO_MANY_REQUESTS,
        Some("project model rate limit exceeded"),
    ));
    assert!(!is_explicit_model_failure(
        StatusCode::TOO_MANY_REQUESTS,
        Some("organization model quota exceeded"),
    ));
    assert!(!is_explicit_model_failure(
        StatusCode::TOO_MANY_REQUESTS,
        Some("account model limit reached"),
    ));
    assert!(!is_explicit_model_failure(
        StatusCode::TOO_MANY_REQUESTS,
        Some("rate limit reached for IP 203.0.113.1"),
    ));
    assert!(!is_explicit_model_failure(
        StatusCode::TOO_MANY_REQUESTS,
        Some("rate limit reached for endpoint /v1/responses"),
    ));
    assert!(!is_explicit_model_failure_for_model(
        StatusCode::TOO_MANY_REQUESTS,
        Some("rate limit reached for gpt-5.4"),
        Some("gpt-5.5"),
    ));
    assert!(is_explicit_model_failure_for_model(
        StatusCode::TOO_MANY_REQUESTS,
        Some("rate limit reached for gpt-5.5"),
        Some("gpt-5.5"),
    ));
    assert!(!is_explicit_model_failure_for_model(
        StatusCode::TOO_MANY_REQUESTS,
        Some("rate limit reached for model gpt-5.4"),
        Some("gpt-5.5"),
    ));
    assert!(!is_explicit_model_failure_for_model(
        StatusCode::TOO_MANY_REQUESTS,
        Some("rate limit reached for gpt-5.5 quota"),
        Some("gpt-5.5"),
    ));
    assert!(is_explicit_model_failure_for_model(
        StatusCode::TOO_MANY_REQUESTS,
        Some("model rate limit exceeded"),
        Some("gpt-5.5"),
    ));
    assert!(!is_explicit_model_failure_for_model(
        StatusCode::BAD_REQUEST,
        Some("unsupported model: gpt-5.4"),
        Some("gpt-5.5"),
    ));
    assert!(is_explicit_model_failure_for_model(
        StatusCode::BAD_REQUEST,
        Some("unsupported model: gpt-5.5"),
        Some("gpt-5.5"),
    ));
    assert!(is_explicit_model_failure_for_model(
        StatusCode::BAD_REQUEST,
        Some("unsupported model: foo"),
        Some("foo"),
    ));
    assert!(is_explicit_model_failure_for_model(
        StatusCode::BAD_REQUEST,
        Some("unsupported_model: pool upstream responded with 400: unsupported model: foo"),
        Some("foo"),
    ));
}

#[test]
fn model_route_failure_messages_are_sanitized_before_persistence() {
    let raw = format!("\u{0000}{}", "upstream model failure ".repeat(40));
    let sanitized = sanitize_account_action_message(&raw).expect("non-empty sanitized message");
    assert!(sanitized.len() <= 240);
    assert!(!sanitized.chars().any(char::is_control));
}

async fn insert_model_failure_attempt(
    state: &AppState,
    account_id: i64,
    invoke_id: &str,
    model: Option<&str>,
) -> i64 {
    sqlx::query(
        "INSERT INTO pool_upstream_request_attempts (invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, status) VALUES (?1, ?2, '/v1/responses', 'pool', ?3, ?4, 'route', 1, 1, 0, 'failed')",
    )
    .bind(invoke_id)
    .bind(format_utc_iso(Utc::now()))
    .bind(model)
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("insert temporary model failure attempt")
    .last_insert_rowid()
}

#[tokio::test]
async fn model_routing_timeline_queries_use_epoch_and_latest_event_indexes() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    crate::ensure_schema(&state.pool)
        .await
        .expect("timeline schema migration should be idempotent");
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Timeline index account",
        "timeline-index-api-key",
        None,
        None,
    )
    .await;
    let model = "gpt-timeline-index";
    let now = Utc::now();
    let recent_local = format_naive(now.with_timezone(&Shanghai).naive_local());
    let recent_utc = format_utc_iso(now - ChronoDuration::seconds(1));
    let expired_utc = format_utc_iso(now - ChronoDuration::minutes(20));
    let expired_local = format_naive(
        (now - ChronoDuration::minutes(20))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let attempt_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id,
            upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index,
            status
        ) VALUES (?1, ?2, '/v1/responses', 'pool', ?3, ?4, 'timeline-index', 1, 1, 0, 'success')
        RETURNING id
        "#,
    )
    .bind("timeline-index-attempt")
    .bind(&recent_local)
    .bind(model)
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("insert local-timestamp timeline attempt");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_events (
            account_id, occurred_at, action, source, attempt_id, model,
            model_route_state_before, model_route_state_after, created_at
        ) VALUES
            (?1, ?2, 'model_route_state_changed', 'call', ?3, ?4, 'available', 'degraded', ?2),
            (?1, ?5, 'model_route_state_changed', 'call', ?3, ?4, 'degraded', 'available', ?5),
            (?1, ?6, 'model_route_state_changed', 'call', NULL, ?4, 'available', 'degraded', ?6),
            (?1, ?7, 'model_route_state_changed', 'call', NULL, ?4, 'available', 'degraded', ?7)
        "#,
    )
    .bind(account_id)
    .bind(&recent_local)
    .bind(attempt_id)
    .bind(model)
    .bind(&recent_utc)
    .bind(&recent_utc)
    .bind(&expired_local)
    .execute(&state.pool)
    .await
    .expect("insert linked and standalone timeline events");

    let cutoff_epoch_ms = (Utc::now() - ChronoDuration::minutes(15)).timestamp_millis();
    let attempt_plan = build_model_routing_attempt_timeline_query(
        "EXPLAIN QUERY PLAN ",
        None,
        None,
        cutoff_epoch_ms,
        10,
        None,
        None,
    )
    .build_query_as::<(i64, i64, i64, String)>()
    .fetch_all(&state.pool)
    .await
    .expect("explain production attempt timeline query")
    .into_iter()
    .map(|(_, _, _, detail)| detail)
    .collect::<Vec<_>>();
    assert!(
        attempt_plan.iter().any(|detail| {
            detail.contains(
                "SEARCH attempts USING INDEX idx_pool_upstream_request_attempts_timeline_epoch",
            )
        }),
        "attempt timeline query must use the epoch range index: {attempt_plan:?}"
    );
    assert!(
        attempt_plan.iter().any(|detail| {
            detail.contains(
                "SEARCH latest USING INDEX idx_pool_upstream_account_events_attempt_latest",
            )
        }),
        "latest event subquery must use the attempt index: {attempt_plan:?}"
    );
    assert!(
        attempt_plan
            .iter()
            .all(|detail| !detail.contains("TEMP B-TREE")),
        "attempt timeline query must not sort through a temporary B-tree: {attempt_plan:?}"
    );

    let event_plan = build_model_routing_event_timeline_query(
        "EXPLAIN QUERY PLAN ",
        None,
        None,
        cutoff_epoch_ms,
        10,
        None,
        None,
    )
    .build_query_as::<(i64, i64, i64, String)>()
    .fetch_all(&state.pool)
    .await
    .expect("explain production standalone event timeline query")
    .into_iter()
    .map(|(_, _, _, detail)| detail)
    .collect::<Vec<_>>();
    assert!(
        event_plan.iter().any(|detail| {
            detail.contains(
                "SEARCH event USING INDEX idx_pool_upstream_account_events_timeline_unlinked_epoch",
            )
        }),
        "standalone event timeline query must use the epoch range index: {event_plan:?}"
    );
    assert!(
        event_plan
            .iter()
            .all(|detail| !detail.contains("TEMP B-TREE")),
        "standalone event timeline query must not sort through a temporary B-tree: {event_plan:?}"
    );

    let Json(live) = get_model_routing_live(
        State(state),
        Query(ModelRoutingLiveQuery {
            window: Some("15m".to_string()),
            model: Some(model.to_string()),
            state: None,
            limit: Some(10),
        }),
    )
    .await
    .expect("load mixed-format model routing timeline");
    assert!(
        live.records
            .iter()
            .any(|record| record.id == format!("attempt:{attempt_id}")),
        "the local-timestamp attempt should remain visible in the 15-minute window"
    );
    assert_eq!(
        live.records
            .iter()
            .find(|record| record.id == format!("attempt:{attempt_id}"))
            .and_then(|record| record.model_route_state_after.as_deref()),
        Some("degraded"),
        "the attempt must use its latest linked transition across timestamp formats"
    );
    assert!(
        live.records
            .iter()
            .any(|record| record.kind == "event" && record.occurred_at == recent_utc),
        "the RFC3339 standalone event should remain visible in the 15-minute window"
    );
    assert!(
        live.records
            .iter()
            .all(|record| record.occurred_at != expired_utc),
        "the expired standalone event must remain outside the 15-minute window"
    );
}

#[tokio::test]
async fn model_routing_live_api_lists_api_key_attempts_and_pages_account_history() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Routing live API account",
        "routing-live-api-key",
        None,
        None,
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET group_name = ?2 WHERE id = ?1")
        .bind(account_id)
        .bind("irrelevant-to-routing")
        .execute(&state.pool)
        .await
        .expect("assign account group outside the routing read model");
    let model = "gpt-routing-live-api";
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed API Key model route");
    let cache_usage_missing_since = format_utc_iso(Utc::now());
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET last_failure_kind = 'upstream_http_5xx', last_failure_message = ?3, cache_usage_missing_since = ?4, cache_usage_missing_reason = 'missing_cache_input_tokens' WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .bind("upstream response contained a sensitive diagnostic")
    .bind(&cache_usage_missing_since)
    .execute(&state.pool)
    .await
    .expect("seed a sensitive failure message");
    let first_attempt =
        insert_model_failure_attempt(&state, account_id, "routing-live-api-first", Some(model))
            .await;
    let second_attempt =
        insert_model_failure_attempt(&state, account_id, "routing-live-api-second", Some(model))
            .await;

    let Json(live) = get_model_routing_live(
        State(state.clone()),
        Query(ModelRoutingLiveQuery {
            window: Some("1h".to_string()),
            model: Some(model.to_string()),
            state: Some(MODEL_ROUTE_STATE_AVAILABLE.to_string()),
            limit: Some(100),
        }),
    )
    .await
    .expect("load live model routing snapshot");
    assert_eq!(live.groups.len(), 1);
    assert_eq!(live.groups[0].model, model);
    assert_eq!(live.groups[0].accounts.len(), 1);
    assert_eq!(live.groups[0].accounts[0].account_id, account_id);
    assert_eq!(
        live.groups[0].accounts[0].route.cache_usage_missing_since,
        Some(cache_usage_missing_since)
    );
    assert_eq!(
        live.groups[0].accounts[0]
            .route
            .cache_usage_missing_reason
            .as_deref(),
        Some("missing_cache_input_tokens")
    );
    assert_eq!(live.records.len(), 2);
    assert!(live.records.iter().all(|record| record.kind == "attempt"));
    assert!(live.records.iter().all(|record| record.model == model));
    let live_json = serde_json::to_value(&live).expect("serialize routing live response");
    assert!(
        live_json
            .pointer("/groups/0/accounts/0/accountGroupName")
            .is_none()
    );
    assert!(
        live_json
            .pointer("/groups/0/accounts/0/lastFailureMessage")
            .is_none()
    );
    assert!(live_json.pointer("/records/0/accountGroupName").is_none());

    let available_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Routing live API available account",
        "routing-live-api-available-key",
        None,
        None,
    )
    .await;
    observe_model_route_seen(&state.pool, available_account_id, Some(model))
        .await
        .expect("seed available API Key model route");
    insert_model_failure_attempt(
        &state,
        available_account_id,
        "routing-live-api-available-attempt",
        Some(model),
    )
    .await;
    let deleted_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Routing live API deleted account",
        "routing-live-api-deleted-key",
        None,
        None,
    )
    .await;
    observe_model_route_seen(&state.pool, deleted_account_id, Some(model))
        .await
        .expect("seed deleted API Key model route");
    insert_model_failure_attempt(
        &state,
        deleted_account_id,
        "routing-live-api-deleted-attempt",
        Some(model),
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET deleted_at = datetime('now') WHERE id = ?1")
        .bind(deleted_account_id)
        .execute(&state.pool)
        .await
        .expect("soft-delete API Key routing account");

    let Json(with_deleted) = get_model_routing_live(
        State(state.clone()),
        Query(ModelRoutingLiveQuery {
            window: Some("1h".to_string()),
            model: Some(model.to_string()),
            state: None,
            limit: Some(100),
        }),
    )
    .await
    .expect("exclude soft-deleted API Key routing account");
    assert!(
        with_deleted
            .groups
            .iter()
            .flat_map(|group| group.accounts.iter())
            .all(|account| account.account_id != deleted_account_id)
    );
    assert!(
        with_deleted
            .records
            .iter()
            .all(|record| record.account_id != deleted_account_id)
    );

    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET state = ?3, priority = ?4 WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .bind(MODEL_ROUTE_STATE_DEGRADED)
    .bind(MODEL_ROUTE_PRIORITY_DEMOTED)
    .execute(&state.pool)
    .await
    .expect("mark route degraded");

    let Json(degraded) = get_model_routing_live(
        State(state.clone()),
        Query(ModelRoutingLiveQuery {
            window: Some("1h".to_string()),
            model: Some(model.to_string()),
            state: Some(MODEL_ROUTE_STATE_DEGRADED.to_string()),
            limit: Some(100),
        }),
    )
    .await
    .expect("filter live route decisions by current state");
    assert_eq!(degraded.groups.len(), 1);
    assert_eq!(degraded.groups[0].accounts.len(), 1);
    assert_eq!(degraded.groups[0].accounts[0].account_id, account_id);
    assert_eq!(degraded.records.len(), 2);
    assert!(
        degraded
            .records
            .iter()
            .all(|record| record.account_id == account_id)
    );

    let Json(first_page) = list_upstream_account_model_routing_events(
        State(state.clone()),
        AxumPath(account_id),
        Query(ModelRoutingHistoryQuery {
            model: model.to_string(),
            cursor: None,
            page_size: Some(1),
        }),
    )
    .await
    .expect("load first model routing history page");
    assert_eq!(first_page.items.len(), 1);
    let history_json =
        serde_json::to_value(&first_page).expect("serialize routing history response");
    assert!(history_json.pointer("/items/0/accountGroupName").is_none());
    let cursor = first_page
        .next_cursor
        .expect("two attempts should yield a next page cursor");

    let Json(second_page) = list_upstream_account_model_routing_events(
        State(state),
        AxumPath(account_id),
        Query(ModelRoutingHistoryQuery {
            model: model.to_string(),
            cursor: Some(cursor),
            page_size: Some(1),
        }),
    )
    .await
    .expect("load second model routing history page");
    assert_eq!(second_page.items.len(), 1);
    assert_ne!(first_page.items[0].id, second_page.items[0].id);
    assert!(second_page.next_cursor.is_none());
    let identifiers = [
        first_page.items[0].id.as_str(),
        second_page.items[0].id.as_str(),
    ];
    assert!(identifiers.contains(&format!("attempt:{first_attempt}").as_str()));
    assert!(identifiers.contains(&format!("attempt:{second_attempt}").as_str()));
}

async fn enable_cache_hit_protection(state: &AppState) {
    sqlx::query(
        "UPDATE pool_routing_settings SET cache_hit_protection_enabled = 1, cache_hit_low_rate_threshold_percent = 10, cache_hit_overflow_mode = 'queue' WHERE id = 1",
    )
    .execute(&state.pool)
    .await
    .expect("enable cache-hit protection");
}

async fn cache_hit_route_state(
    state: &AppState,
    account_id: i64,
    model: &str,
) -> (
    String,
    Option<i64>,
    Option<i64>,
    i64,
    i64,
    Option<i64>,
    Option<String>,
) {
    sqlx::query_as(
        "SELECT state, cache_concurrency_limit, cache_recovery_limit, cache_low_hit_streak, cache_cooldown_level, cache_last_hit_rate_percent, cooldown_until FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .fetch_one(&state.pool)
    .await
    .expect("load cache-hit route state")
}

#[tokio::test]
async fn cache_hit_protection_observation_respects_sample_boundary_and_threshold() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Boundary",
        "cache-boundary-key",
        None,
        Some("https://cache-boundary.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-boundary";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed model route");

    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_839),
        Some(0),
        8,
    )
    .await
    .expect("ignore undersized sample");
    let state_after_small = cache_hit_route_state(&state, account_id, model).await;
    assert_eq!(state_after_small.1, None);
    assert_eq!(state_after_small.5, None);

    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(384),
        8,
    )
    .await
    .expect("observe threshold-equal sample");
    let state_after_equal = cache_hit_route_state(&state, account_id, model).await;
    assert_eq!(state_after_equal.0, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(state_after_equal.1, None);
    assert_eq!(state_after_equal.5, Some(10));

    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(383),
        8,
    )
    .await
    .expect("observe low cache-hit sample");
    let state_after_low = cache_hit_route_state(&state, account_id, model).await;
    assert_eq!(state_after_low.0, MODEL_ROUTE_STATE_DEGRADED);
    assert_eq!(state_after_low.1, Some(4));
    assert_eq!(state_after_low.2, Some(8));
    assert_eq!(state_after_low.5, Some(9));
    let visible_route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load visible cache-hit route")
        .into_iter()
        .find(|route| route.model == model)
        .expect("cache-hit route is visible");
    assert_eq!(visible_route.cache_concurrency_limit, Some(4));
    assert_eq!(visible_route.cache_recovery_limit, Some(8));
    assert_eq!(visible_route.cache_last_hit_rate_percent, Some(9));
    assert!(!visible_route.probe_required);

    observe_model_route_cache_hit(&state.pool, account_id, Some(model), None, None, 8)
        .await
        .expect("constrain cache-owned route with missing usage");
    let missing_usage_route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load missing-usage route")
        .into_iter()
        .find(|route| route.model == model)
        .expect("missing-usage route is visible");
    assert_eq!(missing_usage_route.state, MODEL_ROUTE_STATE_DEGRADED);
    assert_eq!(missing_usage_route.cache_concurrency_limit, Some(1));
    assert!(missing_usage_route.cache_usage_missing_since.is_some());
    assert_eq!(
        missing_usage_route.cache_usage_missing_reason.as_deref(),
        Some("missing_input_tokens")
    );
    assert_eq!(
        model_route_concurrency_limit(&state.pool, account_id, Some(model))
            .await
            .expect("load constrained missing-usage limit"),
        Some(1)
    );

    observe_model_route_cache_hit(&state.pool, account_id, Some(model), None, None, 8)
        .await
        .expect("keep the same missing-usage episode constrained");
    let missing_event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_account_events WHERE account_id = ?1 AND action = ?2",
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_CACHE_OBSERVATION_MISSING)
    .fetch_one(&state.pool)
    .await
    .expect("count missing cache-usage events");
    assert_eq!(missing_event_count, 1);

    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(384),
        1,
    )
    .await
    .expect("clear missing-usage marker with a valid sample");
    let observed_route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load route after valid cache observation")
        .into_iter()
        .find(|route| route.model == model)
        .expect("observed route is visible");
    assert!(observed_route.cache_usage_missing_since.is_none());
    assert!(observed_route.cache_usage_missing_reason.is_none());
    assert_eq!(observed_route.cache_concurrency_limit, Some(2));
}

#[tokio::test]
async fn cache_usage_missing_does_not_claim_an_observation_only_route() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Observation Only",
        "cache-observation-only-key",
        None,
        Some("https://cache-observation-only.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-observation-only";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed observation-only route");
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(384),
        1,
    )
    .await
    .expect("record a healthy cache observation");

    observe_model_route_cache_hit(&state.pool, account_id, Some(model), None, None, 1)
        .await
        .expect("ignore missing usage for observation-only route");

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load observation-only route")
        .into_iter()
        .find(|route| route.model == model)
        .expect("observation-only route is visible");
    assert_eq!(route.state, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(route.priority, MODEL_ROUTE_PRIORITY_NORMAL);
    assert_eq!(route.cache_last_hit_rate_percent, Some(10));
    assert_eq!(route.cache_concurrency_limit, None);
    assert!(route.cache_usage_missing_since.is_none());
    assert!(route.cache_usage_missing_reason.is_none());
    let missing_event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_account_events WHERE account_id = ?1 AND model = ?2 AND action = ?3",
    )
    .bind(account_id)
    .bind(model)
    .bind(UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_CACHE_OBSERVATION_MISSING)
    .fetch_one(&state.pool)
    .await
    .expect("count observation-only missing usage events");
    assert_eq!(missing_event_count, 0);
}

#[tokio::test]
async fn initial_no_candidate_persists_invocation_audit_without_upstream_attempt() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: "no-candidate-initial-audit".to_string(),
        occurred_at: format_naive_precise(Utc::now().with_timezone(&Shanghai).naive_local()),
        endpoint: "/v1/responses".to_string(),
        sticky_key: None,
        requester_ip: None,
        upstream_base_url_host: None,
        request_model: Some("gpt-audit".to_string()),
    };
    let audit = PoolRoutingNoCandidateAudit {
        terminal_reason_code: "modelConcurrencyLimit".to_string(),
        candidate_count: 2,
        eligible_candidate_count: 2,
        reservation_conflict_count: 2,
        next_eligible_at: None,
        excluded_reason_counts: std::collections::BTreeMap::from([(
            "modelConcurrencyLimit".to_string(),
            2,
        )]),
        candidates: vec![PoolRoutingNoCandidateAuditCandidate {
            account_id: 41,
            account_name: "dzw".to_string(),
            reason_code: "modelConcurrencyLimit".to_string(),
        }],
    };

    let request_body = Bytes::from_static(br#"{"model":"gpt-audit","input":[]}"#);
    let error = unwrap_via_pool_initial_account_with_request_body(
        state.clone(),
        Some(&trace),
        Ok(PoolAccountResolutionWithWait::Resolution(
            PoolAccountResolution::NoCandidate(audit.clone()),
        )),
        None,
        None,
        None,
        false,
        None,
        Some(PoolReplayBodySnapshot::Memory(request_body.clone())),
    )
    .await
    .expect_err("no candidate should remain a local terminal failure");
    assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind(&trace.invoke_id)
            .fetch_one(&state.pool)
            .await
            .expect("load persisted no-candidate invocation payload");
    let payload: serde_json::Value = serde_json::from_str(&payload).expect("valid payload");
    assert_eq!(payload["poolAttemptCount"], 0);
    assert_eq!(
        payload["poolRoutingNoCandidateAudit"]["terminalReasonCode"],
        "modelConcurrencyLimit"
    );
    let request_raw_size: i64 = sqlx::query_scalar(
        "SELECT COALESCE(request_raw_size, 0) FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind(&trace.invoke_id)
    .fetch_one(&state.pool)
    .await
    .expect("load persisted no-candidate request body size");
    assert_eq!(request_raw_size, request_body.len() as i64);
    let request_raw_path: Option<String> =
        sqlx::query_scalar("SELECT request_raw_path FROM codex_invocations WHERE invoke_id = ?1")
            .bind(&trace.invoke_id)
            .fetch_one(&state.pool)
            .await
            .expect("load persisted no-candidate request body path");
    assert!(
        request_raw_path.is_some(),
        "zero-attempt diagnostics must retain the known request body instead of a tombstone"
    );
    let attempt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_request_attempts WHERE invoke_id = ?1",
    )
    .bind(&trace.invoke_id)
    .fetch_one(&state.pool)
    .await
    .expect("count upstream attempts");
    assert_eq!(attempt_count, 0);
}

#[tokio::test]
async fn disabling_cache_hit_protection_clears_only_cache_owned_route_state() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Settings Cleanup",
        "cache-settings-cleanup-key",
        None,
        Some("https://cache-settings-cleanup.example.com/backend-api/codex"),
    )
    .await;
    let cache_model = "gpt-cache-settings-cleanup";
    let missing_only_model = "gpt-cache-settings-missing-only";
    let failure_model = "gpt-cache-settings-non-cache-failure";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, account_id, Some(cache_model))
        .await
        .expect("seed cache model route");
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(cache_model),
        Some(3_840),
        Some(0),
        2,
    )
    .await
    .expect("create cache-owned protection state");
    observe_model_route_cache_hit(&state.pool, account_id, Some(cache_model), None, None, 1)
        .await
        .expect("mark cache-owned route usage unavailable");
    observe_model_route_seen(&state.pool, account_id, Some(missing_only_model))
        .await
        .expect("seed missing-only cache model route");
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET cache_concurrency_limit = 4, cache_recovery_limit = 8 WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(missing_only_model)
    .execute(&state.pool)
    .await
    .expect("seed cache limit without a cache failure");
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(missing_only_model),
        None,
        None,
        1,
    )
    .await
    .expect("mark missing-only cache route usage unavailable");
    let missing_only_before_disable = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load missing-only route before disabling protection")
        .into_iter()
        .find(|route| route.model == missing_only_model)
        .expect("missing-only route is visible");
    assert_eq!(
        missing_only_before_disable.state,
        MODEL_ROUTE_STATE_DEGRADED
    );
    assert_eq!(
        missing_only_before_disable.priority,
        MODEL_ROUTE_PRIORITY_DEMOTED
    );
    assert!(missing_only_before_disable.last_failure_kind.is_none());
    assert!(
        missing_only_before_disable
            .cache_usage_missing_since
            .is_some()
    );
    observe_model_route_seen(&state.pool, account_id, Some(failure_model))
        .await
        .expect("seed independent failed model route");
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET state = 'cooling_down', priority = 'excluded', last_failure_kind = 'upstream_transport_error', last_failure_message = 'transport failure', cooldown_until = ?3 WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(failure_model)
    .bind((Utc::now() + chrono::Duration::seconds(30)).to_rfc3339())
    .execute(&state.pool)
    .await
    .expect("seed non-cache cooldown");

    let Json(updated) = update_pool_routing_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(UpdatePoolRoutingSettingsRequest {
            api_key: None,
            maintenance: None,
            request_compression_algorithm: None,
            request_compression_level_preset: None,
            codex_imagegen_rewrite_mode: None,
            available_models: None,
            available_models_mode: None,
            timeouts: None,
            cache_hit_protection: Some(UpdateCacheHitProtectionSettingsRequest {
                enabled: Some(false),
                low_hit_rate_threshold_percent: None,
                overflow_mode: None,
            }),
            live_request_streaming: None,
        }),
    )
    .await
    .expect("disable cache-hit protection");
    assert!(!updated.cache_hit_protection.enabled);
    let cache_route = cache_hit_route_state(&state, account_id, cache_model).await;
    assert_eq!(cache_route.0, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(cache_route.1, None);
    assert_eq!(cache_route.2, None);
    assert_eq!(cache_route.3, 0);
    assert_eq!(cache_route.4, 0);
    assert_eq!(cache_route.5, None);
    let cache_route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load cache route after disabling protection")
        .into_iter()
        .find(|route| route.model == cache_model)
        .expect("cache route is visible");
    assert!(cache_route.cache_usage_missing_since.is_none());
    assert!(cache_route.cache_usage_missing_reason.is_none());
    let missing_only_route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load missing-only route after disabling protection")
        .into_iter()
        .find(|route| route.model == missing_only_model)
        .expect("missing-only route remains visible");
    assert_eq!(missing_only_route.state, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(missing_only_route.priority, MODEL_ROUTE_PRIORITY_NORMAL);
    assert_eq!(missing_only_route.cache_concurrency_limit, None);
    assert_eq!(missing_only_route.cache_recovery_limit, None);
    assert!(missing_only_route.cache_usage_missing_since.is_none());
    assert!(missing_only_route.cache_usage_missing_reason.is_none());
    let non_cache_route = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT state, priority, cooldown_until FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(failure_model)
    .fetch_one(&state.pool)
    .await
    .expect("load non-cache cooldown after cache settings update");
    assert_eq!(non_cache_route.0, MODEL_ROUTE_STATE_COOLING_DOWN);
    assert_eq!(non_cache_route.1, MODEL_ROUTE_PRIORITY_EXCLUDED);
    assert!(non_cache_route.2.is_some());
    let cleanup_event = sqlx::query_as::<_, (String, String, String)>(
        "SELECT action, reason_code, model FROM pool_upstream_account_events WHERE account_id = ?1 AND model = ?2 ORDER BY id DESC LIMIT 1",
    )
    .bind(account_id)
    .bind(cache_model)
    .fetch_one(&state.pool)
    .await
    .expect("load cache settings cleanup event");
    assert_eq!(cleanup_event.0, UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_RESET);
    assert_eq!(cleanup_event.1, "cache_hit_protection_disabled");
    assert_eq!(cleanup_event.2, cache_model);
}

#[tokio::test]
async fn cache_hit_protection_concurrency_halves_then_recovers() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Concurrency",
        "cache-concurrency-key",
        None,
        Some("https://cache-concurrency.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-concurrency";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed model route");

    for expected_limit in [4, 2, 1] {
        observe_model_route_cache_hit(
            &state.pool,
            account_id,
            Some(model),
            Some(3_840),
            Some(0),
            8,
        )
        .await
        .expect("apply low cache-hit protection");
        assert_eq!(
            cache_hit_route_state(&state, account_id, model).await.1,
            Some(expected_limit)
        );
    }
    assert_eq!(cache_hit_route_state(&state, account_id, model).await.3, 1);

    for expected_limit in [2, 3, 4, 5, 6, 7] {
        observe_model_route_cache_hit(
            &state.pool,
            account_id,
            Some(model),
            Some(3_840),
            Some(3_840),
            8,
        )
        .await
        .expect("apply healthy cache-hit observation");
        assert_eq!(
            cache_hit_route_state(&state, account_id, model).await.1,
            Some(expected_limit)
        );
    }
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(3_840),
        8,
    )
    .await
    .expect("fully recover cache-hit route");
    let recovered = cache_hit_route_state(&state, account_id, model).await;
    assert_eq!(recovered.0, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(recovered.1, None);
    assert_eq!(recovered.2, None);
}

#[tokio::test]
async fn cache_hit_protection_serializes_concurrent_observations() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Serialized Observations",
        "cache-serialized-observations-key",
        None,
        Some("https://cache-serialized-observations.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-serialized-observations";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed model route");

    let first = observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(0),
        8,
    );
    let second = observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(0),
        8,
    );
    let (first, second) = tokio::join!(first, second);
    first.expect("first low-hit observation should persist");
    second.expect("second low-hit observation should persist");

    let route = cache_hit_route_state(&state, account_id, model).await;
    assert_eq!(route.1, Some(2));
    assert_eq!(route.2, Some(8));
}

#[tokio::test]
async fn cache_hit_protection_atomically_reserves_single_model_slot() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Reservation",
        "cache-reservation-key",
        None,
        Some("https://cache-reservation.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-reservation";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed model route");
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(0),
        2,
    )
    .await
    .expect("limit the combination to one request");
    assert_eq!(
        model_route_concurrency_limit(&state.pool, account_id, Some(model))
            .await
            .expect("load model concurrency limit"),
        Some(1)
    );
    sqlx::query(
        "UPDATE pool_routing_settings SET cache_hit_overflow_mode = 'reroute' WHERE id = 1",
    )
    .execute(&state.pool)
    .await
    .expect("switch cache-hit overflow mode to reroute");

    let excluded_ids = Vec::new();
    let excluded_routes = HashSet::new();
    let first = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        None,
        Some(model),
        &excluded_ids,
        &excluded_routes,
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("cache-hit-reservation-a"),
    );
    let second = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        None,
        Some(model),
        &excluded_ids,
        &excluded_routes,
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("cache-hit-reservation-b"),
    );
    let (first, second) = tokio::join!(first, second);
    let resolutions = [
        first.expect("first selection should complete"),
        second.expect("second selection should complete"),
    ];
    assert_eq!(
        resolutions
            .iter()
            .filter(|resolution| matches!(resolution, PoolAccountResolution::Resolved(_)))
            .count(),
        1
    );
    assert_eq!(
        resolutions
            .iter()
            .filter(|resolution| matches!(resolution, PoolAccountResolution::NoCandidate(_)))
            .count(),
        1
    );
    let audit = resolutions
        .iter()
        .find_map(|resolution| match resolution {
            PoolAccountResolution::NoCandidate(audit) => Some(audit),
            _ => None,
        })
        .expect("capacity conflict should retain a no-candidate audit");
    assert_eq!(audit.terminal_reason_code, "modelConcurrencyLimit");
    assert_eq!(audit.candidate_count, 1);
    assert_eq!(audit.eligible_candidate_count, 1);
    assert_eq!(audit.reservation_conflict_count, 1);
    assert_eq!(audit.candidates[0].reason_code, "modelConcurrencyLimit");

    release_pool_routing_reservation(&state, "cache-hit-reservation-a");
    release_pool_routing_reservation(&state, "cache-hit-reservation-b");
}

#[tokio::test]
async fn cache_hit_protection_reserves_sticky_fast_path_before_returning_it() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Sticky Reservation",
        "cache-sticky-reservation-key",
        None,
        Some("https://cache-sticky-reservation.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-sticky-reservation";
    let sticky_key = "cache-hit-sticky-reservation";
    enable_cache_hit_protection(&state).await;
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
    .expect("seed sticky model concurrency limit");
    upsert_sticky_route(
        &state.pool,
        sticky_key,
        account_id,
        &format_utc_iso(Utc::now()),
    )
    .await
    .expect("seed sticky route");

    let excluded_ids = Vec::new();
    let excluded_routes = HashSet::new();
    let first = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        Some(sticky_key),
        Some(model),
        &excluded_ids,
        &excluded_routes,
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("cache-hit-sticky-reservation-a"),
    );
    let second = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        Some(sticky_key),
        Some(model),
        &excluded_ids,
        &excluded_routes,
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("cache-hit-sticky-reservation-b"),
    );
    let (first, second) = tokio::join!(first, second);
    let resolutions = [
        first.expect("first sticky selection should complete"),
        second.expect("second sticky selection should complete"),
    ];
    assert_eq!(
        resolutions
            .iter()
            .filter(|resolution| matches!(resolution, PoolAccountResolution::Resolved(_)))
            .count(),
        1
    );
    assert_eq!(
        resolutions
            .iter()
            .filter(|resolution| matches!(resolution, PoolAccountResolution::NoCandidate(_)))
            .count(),
        1
    );

    release_pool_routing_reservation(&state, "cache-hit-sticky-reservation-a");
    release_pool_routing_reservation(&state, "cache-hit-sticky-reservation-b");
}

#[tokio::test]
async fn cache_hit_protection_reroutes_a_capped_normal_sticky_route() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let sticky_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Sticky Reroute Source",
        "cache-sticky-reroute-source-key",
        None,
        Some("https://cache-sticky-reroute-source.example.com/backend-api/codex"),
    )
    .await;
    let alternative_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Sticky Reroute Alternative",
        "cache-sticky-reroute-alternative-key",
        None,
        Some("https://cache-sticky-reroute-alternative.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-sticky-reroute";
    let sticky_key = "cache-hit-sticky-reroute";
    enable_cache_hit_protection(&state).await;
    sqlx::query(
        "UPDATE pool_routing_settings SET cache_hit_overflow_mode = 'reroute' WHERE id = 1",
    )
    .execute(&state.pool)
    .await
    .expect("enable cache-hit reroute mode");
    observe_model_route_seen(&state.pool, sticky_account_id, Some(model))
        .await
        .expect("seed sticky model route");
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET cache_concurrency_limit = 1, cache_recovery_limit = 2 WHERE account_id = ?1 AND model = ?2",
    )
    .bind(sticky_account_id)
    .bind(model)
    .execute(&state.pool)
    .await
    .expect("seed sticky model concurrency limit");
    upsert_sticky_route(
        &state.pool,
        sticky_key,
        sticky_account_id,
        &format_utc_iso(Utc::now()),
    )
    .await
    .expect("seed reroutable sticky route");

    let excluded_ids = Vec::new();
    let excluded_routes = HashSet::new();
    let first = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        Some(sticky_key),
        Some(model),
        &excluded_ids,
        &excluded_routes,
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("cache-hit-sticky-reroute-a"),
    )
    .await
    .expect("reserve normal sticky route");
    let PoolAccountResolution::Resolved(first) = first else {
        panic!("expected sticky route to be selected before its cap is full");
    };
    assert_eq!(first.account_id, sticky_account_id);

    let second = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        Some(sticky_key),
        Some(model),
        &excluded_ids,
        &excluded_routes,
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("cache-hit-sticky-reroute-b"),
    )
    .await
    .expect("reroute after sticky model cap is full");
    let PoolAccountResolution::Resolved(second) = second else {
        panic!("expected a legal alternative after sticky model cap is full");
    };
    assert_eq!(second.account_id, alternative_account_id);

    release_pool_routing_reservation(&state, "cache-hit-sticky-reroute-a");
    release_pool_routing_reservation(&state, "cache-hit-sticky-reroute-b");
}

#[tokio::test]
async fn model_route_reservation_keeps_resolved_model_when_retry_context_lacks_one() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Retry Reservation",
        "cache-retry-reservation-key",
        None,
        Some("https://cache-retry-reservation.example.com/backend-api/codex"),
    )
    .await;
    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve account for reservation");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected a pool account for reservation");
    };
    assert_eq!(account.account_id, account_id);

    reserve_pool_routing_account_for_model(
        &state,
        "cache-retry-reservation",
        &account,
        Some("gpt-cache-retry-reservation"),
    );
    reserve_pool_routing_account_for_model(&state, "cache-retry-reservation", &account, None);
    assert!(pool_routing_reservation_matches_model(
        &state,
        "cache-retry-reservation",
        account_id,
        Some("gpt-cache-retry-reservation"),
    ));

    release_pool_routing_reservation(&state, "cache-retry-reservation");
}

#[tokio::test]
async fn model_route_reservation_preserves_an_explicit_empty_model() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Empty Model Reservation",
        "empty-model-reservation-key",
        None,
        Some("https://empty-model-reservation.example.com/backend-api/codex"),
    )
    .await;
    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve account for empty model reservation");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected a pool account for empty model reservation");
    };
    assert_eq!(account.account_id, account_id);

    reserve_pool_routing_account_for_model(&state, "empty-model-reservation", &account, Some(""));
    assert_eq!(
        pool_routing_model_reservation_count(&state, account_id, Some("")),
        1
    );
    assert!(pool_routing_reservation_matches_model(
        &state,
        "empty-model-reservation",
        account_id,
        Some(""),
    ));

    release_pool_routing_reservation(&state, "empty-model-reservation");
}

#[test]
fn websocket_terminal_reservation_key_reuses_the_active_pool_route_key() {
    assert_eq!(
        pool_routing_reservation_key_for_invoke_id("pool-ws-42-turn-3").as_deref(),
        Some("pool-route-42")
    );
}

#[tokio::test]
async fn observing_a_stale_model_route_clears_cache_hit_protection_state() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Stale Route",
        "cache-stale-route-key",
        None,
        Some("https://cache-stale-route.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-stale-route";
    let stale_at = format_utc_iso(Utc::now() - chrono::Duration::days(8));
    sqlx::query(
        "INSERT INTO pool_upstream_account_model_routes (account_id, model, state, priority, consecutive_failures, changed_at, last_seen_at, cache_concurrency_limit, cache_recovery_limit, cache_low_hit_streak, cache_cooldown_level, cache_last_hit_rate_percent) VALUES (?1, ?2, 'degraded', 'demoted', 2, ?3, ?3, 1, 8, 2, 3, 0)",
    )
    .bind(account_id)
    .bind(model)
    .bind(&stale_at)
    .execute(&state.pool)
    .await
    .expect("seed stale cache-hit route state");

    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("observe stale model route");
    let route = cache_hit_route_state(&state, account_id, model).await;
    assert_eq!(route.0, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(route.1, None);
    assert_eq!(route.2, None);
    assert_eq!(route.3, 0);
    assert_eq!(route.4, 0);
    assert_eq!(route.5, None);
}

#[tokio::test]
async fn cache_hit_protection_reroutes_to_another_legal_combination() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let limited_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Reroute Limited",
        "cache-reroute-limited-key",
        None,
        Some("https://cache-reroute-limited.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-reroute";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, limited_account_id, Some(model))
        .await
        .expect("seed limited model route");
    observe_model_route_cache_hit(
        &state.pool,
        limited_account_id,
        Some(model),
        Some(3_840),
        Some(0),
        2,
    )
    .await
    .expect("limit first account to one request");

    let excluded_ids = Vec::new();
    let excluded_routes = HashSet::new();
    let busy = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        None,
        Some(model),
        &excluded_ids,
        &excluded_routes,
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("cache-reroute-busy"),
    )
    .await
    .expect("reserve the limited account");
    let PoolAccountResolution::Resolved(busy) = busy else {
        panic!("limited account should be selected while it has capacity");
    };
    assert_eq!(busy.account_id, limited_account_id);

    let alternative_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Reroute Alternative",
        "cache-reroute-alternative-key",
        None,
        Some("https://cache-reroute-alternative.example.com/backend-api/codex"),
    )
    .await;
    sqlx::query(
        "UPDATE pool_routing_settings SET cache_hit_overflow_mode = 'reroute' WHERE id = 1",
    )
    .execute(&state.pool)
    .await
    .expect("switch cache-hit overflow mode to reroute");

    let rerouted = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        None,
        Some(model),
        &excluded_ids,
        &excluded_routes,
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("cache-reroute-alternative"),
    )
    .await
    .expect("reroute selection should complete");
    let PoolAccountResolution::Resolved(rerouted) = rerouted else {
        panic!("an alternate account should remain selectable");
    };
    assert_eq!(rerouted.account_id, alternative_account_id);

    release_pool_routing_reservation(&state, "cache-reroute-busy");
    release_pool_routing_reservation(&state, "cache-reroute-alternative");
}

#[tokio::test]
async fn model_route_single_probe_recovery_is_atomic_for_non_cache_cooldowns() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Single Probe Recovery",
        "single-probe-recovery-key",
        None,
        Some("https://single-probe-recovery.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-single-probe-recovery";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed model route");
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET state = 'cooling_down', priority = 'excluded', cooldown_until = ?3, last_failure_kind = 'http_5xx', last_failure_message = 'upstream unavailable' WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .bind((Utc::now() - chrono::Duration::seconds(1)).to_rfc3339())
    .execute(&state.pool)
    .await
    .expect("expire non-cache model cooldown");
    assert_eq!(
        model_route_concurrency_limit(&state.pool, account_id, Some(model))
            .await
            .expect("load non-cache probe limit"),
        Some(1)
    );

    let excluded_ids = Vec::new();
    let excluded_routes = HashSet::new();
    let first = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        None,
        Some(model),
        &excluded_ids,
        &excluded_routes,
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("single-probe-reservation-a"),
    );
    let second = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        None,
        Some(model),
        &excluded_ids,
        &excluded_routes,
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("single-probe-reservation-b"),
    );
    let (first, second) = tokio::join!(first, second);
    let resolutions = [
        first.expect("first probe selection should complete"),
        second.expect("second probe selection should complete"),
    ];
    assert_eq!(
        resolutions
            .iter()
            .filter(|resolution| matches!(resolution, PoolAccountResolution::Resolved(_)))
            .count(),
        1
    );
    assert_eq!(
        resolutions
            .iter()
            .filter(|resolution| matches!(resolution, PoolAccountResolution::NoCandidate(_)))
            .count(),
        1
    );

    release_pool_routing_reservation(&state, "single-probe-reservation-a");
    release_pool_routing_reservation(&state, "single-probe-reservation-b");

    let success_started_at =
        format_naive_precise(Utc::now().with_timezone(&Shanghai).naive_local());
    let successful_attempt = sqlx::query(
        "INSERT INTO pool_upstream_request_attempts (invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, started_at, status) VALUES (?1, ?2, '/v1/responses', 'pool', ?3, ?4, 'route', 1, 1, 0, ?5, 'pending')",
    )
    .bind("non-cache-expired-cooldown-success")
    .bind(format_utc_iso(Utc::now()))
    .bind(model)
    .bind(account_id)
    .bind(&success_started_at)
    .execute(&state.pool)
    .await
    .expect("insert successful non-cache probe")
    .last_insert_rowid();
    record_model_route_success_from_attempt(
        &state.pool,
        account_id,
        successful_attempt,
        Some(&success_started_at),
    )
    .await
    .expect("recover successful non-cache probe without cache usage");
    let recovered = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        "SELECT state, last_failure_kind, cooldown_until FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .fetch_one(&state.pool)
    .await
    .expect("load recovered non-cache model route");
    assert_eq!(recovered.0, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(
        model_route_concurrency_limit(&state.pool, account_id, Some(model))
            .await
            .expect("load recovered model concurrency"),
        None
    );
    assert!(recovered.1.is_none());
    assert!(recovered.2.is_none());
}

#[tokio::test]
async fn cache_hit_protection_cooldown_restarts_with_single_probe() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Cooldown",
        "cache-cooldown-key",
        None,
        Some("https://cache-cooldown.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-cooldown";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed model route");

    for _ in 0..5 {
        observe_model_route_cache_hit(
            &state.pool,
            account_id,
            Some(model),
            Some(3_840),
            Some(0),
            8,
        )
        .await
        .expect("apply low cache-hit protection");
    }
    let first_cooldown = cache_hit_route_state(&state, account_id, model).await;
    assert_eq!(first_cooldown.0, MODEL_ROUTE_STATE_COOLING_DOWN);
    assert_eq!(first_cooldown.3, 0);
    assert_eq!(first_cooldown.4, 1);
    assert!(first_cooldown.6.is_some());

    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET cooldown_until = ?3 WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .bind((Utc::now() - chrono::Duration::seconds(1)).to_rfc3339())
    .execute(&state.pool)
    .await
    .expect("expire cooldown");
    assert_eq!(
        model_route_concurrency_limit(&state.pool, account_id, Some(model))
            .await
            .expect("load expired cooldown probe limit"),
        Some(1)
    );
    assert!(
        load_model_routing_states(&state.pool, account_id)
            .await
            .expect("load expired cache-hit route")
            .into_iter()
            .find(|route| route.model == model)
            .expect("expired cache-hit route is visible")
            .probe_required
    );

    observe_model_route_cache_hit(&state.pool, account_id, Some(model), None, None, 1)
        .await
        .expect("unknown probe observation");
    let missing_usage_route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load expired route with missing cache usage")
        .into_iter()
        .find(|route| route.model == model)
        .expect("missing-usage route is visible");
    assert_eq!(missing_usage_route.state, MODEL_ROUTE_STATE_DEGRADED);
    assert_eq!(missing_usage_route.cache_concurrency_limit, Some(1));
    assert!(missing_usage_route.cache_usage_missing_since.is_some());
    assert_eq!(
        missing_usage_route.cache_usage_missing_reason.as_deref(),
        Some("missing_input_tokens")
    );

    for expected_streak in [1, 2] {
        observe_model_route_cache_hit(
            &state.pool,
            account_id,
            Some(model),
            Some(3_840),
            Some(0),
            1,
        )
        .await
        .expect("apply low probe observation");
        let state_after_low = cache_hit_route_state(&state, account_id, model).await;
        assert_eq!(state_after_low.0, MODEL_ROUTE_STATE_DEGRADED);
        assert_eq!(state_after_low.1, Some(1));
        assert_eq!(state_after_low.3, expected_streak);
    }
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(0),
        1,
    )
    .await
    .expect("re-enter cache cooldown");
    let second_cooldown = cache_hit_route_state(&state, account_id, model).await;
    assert_eq!(second_cooldown.0, MODEL_ROUTE_STATE_COOLING_DOWN);
    assert_eq!(second_cooldown.4, 2);
}

#[tokio::test]
async fn api_key_temporary_http_failure_changes_only_the_exact_model_route() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Temporary model scope",
        "temporary-model-scope-key",
        None,
        Some("https://temporary-model-scope.example.com/backend-api/codex"),
    )
    .await;
    upsert_sticky_route(
        &state.pool,
        "sticky-temporary-model-scope",
        account_id,
        &format_utc_iso(Utc::now()),
    )
    .await
    .expect("seed temporary model sticky route");
    let attempt_id = insert_model_failure_attempt(
        &state,
        account_id,
        "temporary-model-502",
        Some("gpt-5.6-terra"),
    )
    .await;

    record_pool_route_http_failure_for_endpoint_with_image_intent_for_attempt(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
        true,
        Some("sticky-temporary-model-scope"),
        StatusCode::BAD_GATEWAY,
        "pool upstream responded with 502: Upstream access forbidden, please contact administrator",
        Some("temporary-model-502"),
        "/v1/responses",
        ImageIntent::Unknown,
        Some(attempt_id),
        None,
    )
    .await
    .expect("record API key temporary HTTP failure");

    let account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load API key account")
        .expect("API key account exists");
    assert_eq!(account.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(account.last_route_failure_kind.is_none());
    assert!(account.cooldown_until.is_none());
    assert_eq!(account.consecutive_route_failures, 0);
    assert!(account.temporary_route_failure_streak_started_at.is_none());
    assert_eq!(
        load_sticky_route(&state.pool, "sticky-temporary-model-scope")
            .await
            .expect("load temporary model sticky route")
            .map(|route| route.account_id),
        Some(account_id),
    );

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load temporary model route")
        .into_iter()
        .find(|route| route.model == "gpt-5.6-terra")
        .expect("temporary model route exists");
    assert_eq!(route.state, MODEL_ROUTE_STATE_DEGRADED);
    assert_eq!(route.priority, MODEL_ROUTE_PRIORITY_DEMOTED);
    assert_eq!(route.failure_count, 1);
    assert_eq!(
        route.last_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX),
    );

    let event = sqlx::query_as::<_, (String, Option<String>, Option<i64>, Option<String>, Option<String>)>(
        "SELECT action, reason_code, http_status, failure_kind, model FROM pool_upstream_account_events WHERE account_id = ?1 ORDER BY id DESC LIMIT 1",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load temporary model event");
    assert_eq!(event.0, UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_DEGRADED);
    assert_eq!(
        event.1.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_5XX),
    );
    assert_eq!(event.2, Some(502));
    assert_eq!(
        event.3.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX)
    );
    assert_eq!(event.4.as_deref(), Some("gpt-5.6-terra"));
}

#[tokio::test]
async fn api_key_transport_failure_preserves_kind_and_changes_only_the_exact_model_route() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Temporary transport model scope",
        "temporary-transport-model-key",
        None,
        Some("https://temporary-transport-model.example.com/backend-api/codex"),
    )
    .await;
    let attempt_id = insert_model_failure_attempt(
        &state,
        account_id,
        "temporary-model-stream",
        Some("gpt-5.6-terra"),
    )
    .await;

    record_pool_route_transport_failure_for_attempt_with_kind(
        &state.pool,
        account_id,
        None,
        "upstream stream error before first chunk",
        PROXY_FAILURE_UPSTREAM_STREAM_ERROR,
        Some("temporary-model-stream"),
        Some(attempt_id),
    )
    .await
    .expect("record API key temporary transport failure");

    let account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load API key account")
        .expect("API key account exists");
    assert!(account.last_route_failure_kind.is_none());
    assert!(account.cooldown_until.is_none());
    assert_eq!(account.consecutive_route_failures, 0);

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load transport model route")
        .into_iter()
        .find(|route| route.model == "gpt-5.6-terra")
        .expect("transport model route exists");
    assert_eq!(route.state, MODEL_ROUTE_STATE_DEGRADED);
    assert_eq!(route.priority, MODEL_ROUTE_PRIORITY_DEMOTED);
    assert_eq!(
        route.last_failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR),
    );

    let event = sqlx::query_as::<_, (Option<i64>, Option<String>, Option<String>)>(
        "SELECT attempt_id, failure_kind, model FROM pool_upstream_account_events WHERE account_id = ?1 ORDER BY id DESC LIMIT 1",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load temporary transport model event");
    assert_eq!(event.0, Some(attempt_id));
    assert_eq!(
        event.1.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR)
    );
    assert_eq!(event.2.as_deref(), Some("gpt-5.6-terra"));
}

#[tokio::test]
async fn api_key_pre_attempt_transport_failure_uses_the_exact_request_model() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Pre-attempt transport model scope",
        "pre-attempt-transport-model-key",
        None,
        Some("https://pre-attempt-transport-model.example.com/backend-api/codex"),
    )
    .await;

    record_pool_route_transport_failure_for_model(
        &state.pool,
        account_id,
        None,
        "no selectable forward proxy node",
        Some("pre-attempt-transport-model"),
        Some("gpt-5.6-terra"),
    )
    .await
    .expect("record API key pre-attempt transport failure");

    let account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load API key account")
        .expect("API key account exists");
    assert!(account.last_route_failure_kind.is_none());
    assert!(account.cooldown_until.is_none());
    assert_eq!(account.consecutive_route_failures, 0);

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load pre-attempt transport model route")
        .into_iter()
        .find(|route| route.model == "gpt-5.6-terra")
        .expect("pre-attempt transport model route exists");
    assert_eq!(route.state, MODEL_ROUTE_STATE_DEGRADED);
    assert_eq!(route.priority, MODEL_ROUTE_PRIORITY_DEMOTED);
    assert_eq!(route.failure_count, 1);
    assert_eq!(
        route.last_failure_kind.as_deref(),
        Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM),
    );

    let event = sqlx::query_as::<_, (Option<i64>, Option<String>, Option<String>, Option<String>)>(
        "SELECT attempt_id, reason_code, failure_kind, model FROM pool_upstream_account_events WHERE account_id = ?1 ORDER BY id DESC LIMIT 1",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load pre-attempt transport model event");
    assert_eq!(event.0, None);
    assert_eq!(
        event.1.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE),
    );
    assert_eq!(
        event.2.as_deref(),
        Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM),
    );
    assert_eq!(event.3.as_deref(), Some("gpt-5.6-terra"));
}

#[tokio::test]
async fn api_key_explicit_model_429_honors_toggle_and_preserves_reason() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Explicit model 429",
        "explicit-model-429-key",
        None,
        Some("https://explicit-model-429.example.com/backend-api/codex"),
    )
    .await;
    let enabled_attempt = insert_model_failure_attempt(
        &state,
        account_id,
        "explicit-model-429-enabled",
        Some("gpt-rate-limited"),
    )
    .await;

    record_pool_route_http_failure_for_endpoint_with_image_intent_for_attempt(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
        false,
        None,
        StatusCode::TOO_MANY_REQUESTS,
        "rate limit reached for gpt-rate-limited",
        Some("explicit-model-429-enabled"),
        "/v1/responses",
        ImageIntent::Unknown,
        Some(enabled_attempt),
        None,
    )
    .await
    .expect("record enabled explicit model 429");

    let enabled_event = sqlx::query_as::<_, (Option<String>, Option<i64>, Option<String>)>(
        "SELECT reason_code, http_status, model FROM pool_upstream_account_events WHERE account_id = ?1 ORDER BY id DESC LIMIT 1",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load enabled explicit model 429 event");
    assert_eq!(
        enabled_event.0.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT),
    );
    assert_eq!(enabled_event.1, Some(429));
    assert_eq!(enabled_event.2.as_deref(), Some("gpt-rate-limited"));

    sqlx::query(
        "UPDATE pool_upstream_accounts SET policy_status_change_upstream_http_429_rate_limit = 0 WHERE id = ?1",
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("disable model 429 status changes");
    let disabled_attempt = insert_model_failure_attempt(
        &state,
        account_id,
        "explicit-model-429-disabled",
        Some("gpt-rate-disabled"),
    )
    .await;
    record_pool_route_http_failure_for_endpoint_with_image_intent_for_attempt(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
        false,
        None,
        StatusCode::TOO_MANY_REQUESTS,
        "rate limit reached for gpt-rate-disabled",
        Some("explicit-model-429-disabled"),
        "/v1/responses",
        ImageIntent::Unknown,
        Some(disabled_attempt),
        None,
    )
    .await
    .expect("record disabled explicit model 429");

    assert!(
        load_model_routing_states(&state.pool, account_id)
            .await
            .expect("load explicit model 429 routes")
            .iter()
            .all(|route| route.model != "gpt-rate-disabled")
    );
}

#[tokio::test]
async fn api_key_unattributed_disabled_and_payload_failures_are_evidence_only() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Evidence only temporary failures",
        "evidence-only-temporary-key",
        None,
        Some("https://evidence-only-temporary.example.com/backend-api/codex"),
    )
    .await;

    record_pool_route_http_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
        false,
        None,
        StatusCode::BAD_GATEWAY,
        "pool upstream responded with 502",
        Some("temporary-without-model"),
    )
    .await
    .expect("record unattributed temporary failure");

    let payload_attempt = insert_model_failure_attempt(
        &state,
        account_id,
        "payload-too-large-model",
        Some("gpt-payload"),
    )
    .await;
    record_pool_route_http_failure_for_endpoint_with_image_intent_for_attempt(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
        false,
        None,
        StatusCode::PAYLOAD_TOO_LARGE,
        "request body is too large",
        Some("payload-too-large-model"),
        "/v1/responses",
        ImageIntent::Unknown,
        Some(payload_attempt),
        None,
    )
    .await
    .expect("record payload failure");

    sqlx::query(
        "UPDATE pool_upstream_accounts SET policy_status_change_upstream_http_5xx = 0 WHERE id = ?1",
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("disable upstream 5xx model status changes");
    let disabled_attempt = insert_model_failure_attempt(
        &state,
        account_id,
        "disabled-502-model",
        Some("gpt-disabled"),
    )
    .await;
    record_pool_route_http_failure_for_endpoint_with_image_intent_for_attempt(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
        false,
        None,
        StatusCode::BAD_GATEWAY,
        "pool upstream responded with 502",
        Some("disabled-502-model"),
        "/v1/responses",
        ImageIntent::Unknown,
        Some(disabled_attempt),
        None,
    )
    .await
    .expect("record disabled model status change");

    let account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load evidence-only account")
        .expect("evidence-only account exists");
    assert!(account.last_route_failure_kind.is_none());
    assert!(account.cooldown_until.is_none());
    assert_eq!(account.consecutive_route_failures, 0);
    assert!(
        load_model_routing_states(&state.pool, account_id)
            .await
            .expect("load evidence-only model routes")
            .is_empty()
    );
}

#[tokio::test]
async fn stale_model_failure_does_not_overwrite_newer_success() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Stale model health",
        "stale-model-health-key",
        None,
        Some("https://stale-model-health.example.com/backend-api/codex"),
    )
    .await;

    async fn attempt(
        state: &AppState,
        account_id: i64,
        model: &str,
        started_at: &str,
        status: &str,
    ) -> i64 {
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, started_at, status) VALUES (?1, ?2, '/v1/responses', 'pool', ?3, ?4, 'route', 1, 1, 0, ?5, ?6)",
        )
        .bind(format!("stale-model-{model}-{started_at}"))
        .bind(format_utc_iso(Utc::now()))
        .bind(model)
        .bind(account_id)
        .bind(started_at)
        .bind(status)
        .execute(&state.pool)
        .await
        .expect("insert stale model attempt")
        .last_insert_rowid()
    }

    let now = Utc::now();
    let old_started = format_utc_iso(now - chrono::Duration::seconds(5));
    let newer_started = format_utc_iso(now - chrono::Duration::seconds(1));
    let old_attempt = attempt(&state, account_id, "gpt-stale", &old_started, "failed").await;
    let newer_attempt = attempt(&state, account_id, "gpt-stale", &newer_started, "pending").await;

    record_model_route_success_from_attempt(
        &state.pool,
        account_id,
        newer_attempt,
        Some(&newer_started),
    )
    .await
    .expect("record newer model success");
    record_model_route_failure_from_attempt(
        &state.pool,
        account_id,
        old_attempt,
        StatusCode::BAD_REQUEST,
        Some("model unavailable"),
        Some("model_unavailable"),
    )
    .await
    .expect("ignore stale model failure");

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load stale model route")
        .into_iter()
        .find(|route| route.model == "gpt-stale")
        .expect("stale model route exists");
    assert_eq!(route.failure_count, 0);
    assert_eq!(route.state, MODEL_ROUTE_STATE_AVAILABLE);
    assert!(route.last_failure_at.is_none());
}

#[tokio::test]
async fn later_failed_attempt_does_not_suppress_model_failure() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Later failed model attempt",
        "later-failed-model-attempt-key",
        None,
        Some("https://later-failed-model-attempt.example.com/backend-api/codex"),
    )
    .await;
    let now = Utc::now();
    let local_started_at = |offset_seconds: i64| {
        format_naive_precise(
            (now + ChronoDuration::seconds(offset_seconds))
                .with_timezone(&Shanghai)
                .naive_local(),
        )
    };
    let insert_attempt = |invoke_id: &str, started_at: String, status: &str| {
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, started_at, status) VALUES (?1, ?2, '/v1/responses', 'pool', 'gpt-overlap', ?3, 'route', 1, 1, 0, ?4, ?5)",
        )
        .bind(invoke_id.to_string())
        .bind(format_utc_iso(Utc::now()))
        .bind(account_id)
        .bind(started_at)
        .bind(status.to_string())
    };
    let success_started = local_started_at(-3);
    let success_attempt = insert_attempt("overlap-success", success_started.clone(), "success")
        .execute(&state.pool)
        .await
        .expect("insert overlap success")
        .last_insert_rowid();
    record_model_route_success_from_attempt(
        &state.pool,
        account_id,
        success_attempt,
        Some(&success_started),
    )
    .await
    .expect("record overlap success");

    let failing_started = local_started_at(-2);
    let failing_attempt = insert_attempt("overlap-failure", failing_started, "failed")
        .execute(&state.pool)
        .await
        .expect("insert overlap failure")
        .last_insert_rowid();
    insert_attempt("overlap-later-failure", local_started_at(-1), "failed")
        .execute(&state.pool)
        .await
        .expect("insert later failed overlap attempt");

    record_model_route_failure_from_attempt(
        &state.pool,
        account_id,
        failing_attempt,
        StatusCode::BAD_REQUEST,
        Some("model unavailable"),
        Some("model_unavailable"),
    )
    .await
    .expect("record unsuppressed overlap failure");

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load overlap model route")
        .into_iter()
        .find(|route| route.model == "gpt-overlap")
        .expect("overlap model route exists");
    assert_eq!(route.failure_count, 1);
    assert_eq!(route.state, MODEL_ROUTE_STATE_DEGRADED);
}

#[tokio::test]
async fn observing_model_after_retention_window_resets_dynamic_health() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Expired model observation",
        "expired-model-observation-key",
        None,
        Some("https://expired-model-observation.example.com/backend-api/codex"),
    )
    .await;
    let old = format_utc_iso(Utc::now() - ChronoDuration::days(8));
    let cooldown_until = format_utc_iso(Utc::now() + ChronoDuration::seconds(30));
    sqlx::query(
        "INSERT INTO pool_upstream_account_model_routes (account_id, model, state, priority, consecutive_failures, streak_started_at, changed_at, last_seen_at, last_success_at, last_failure_at, last_failure_kind, last_failure_message, cooldown_until) VALUES (?1, 'gpt-expired-observation', 'cooling_down', 'excluded', 5, ?2, ?2, ?2, ?2, ?2, 'model_unavailable', 'model unavailable', ?3)",
    )
    .bind(account_id)
    .bind(&old)
    .bind(&cooldown_until)
    .execute(&state.pool)
    .await
    .expect("seed expired observation route");

    observe_model_route_seen(&state.pool, account_id, Some("gpt-expired-observation"))
        .await
        .expect("observe expired model route");

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load refreshed model route")
        .into_iter()
        .find(|route| route.model == "gpt-expired-observation")
        .expect("refreshed model route exists");
    assert_eq!(route.state, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(route.priority, MODEL_ROUTE_PRIORITY_NORMAL);
    assert_eq!(route.failure_count, 0);
    assert!(route.last_failure_at.is_none());
    assert!(route.last_failure_kind.is_none());
    assert!(route.last_failure_message.is_none());
    assert!(route.cooldown_until.is_none());
}

#[tokio::test]
async fn precise_attempt_start_preserves_same_second_model_success() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Precise model health",
        "precise-model-health-key",
        None,
        Some("https://precise-model-health.example.com/backend-api/codex"),
    )
    .await;
    let base_epoch = Utc::now().timestamp();
    let failure_at = Utc
        .timestamp_opt(base_epoch, 100_000_000)
        .single()
        .expect("failure timestamp");
    let request_started_at_utc = Utc
        .timestamp_opt(base_epoch, 200_000_000)
        .single()
        .expect("request timestamp");
    let failure_at = format_naive_precise(failure_at.with_timezone(&Shanghai).naive_local());
    let request_started_at = format_naive_precise(
        request_started_at_utc
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    sqlx::query(
        "INSERT INTO pool_upstream_account_model_routes (account_id, model, state, priority, consecutive_failures, streak_started_at, changed_at, last_seen_at, last_failure_at, last_failure_kind, last_failure_message) VALUES (?1, 'gpt-precise', 'degraded', 'demoted', 1, ?2, ?2, ?2, ?2, 'model_unavailable', 'model unavailable')",
    )
    .bind(account_id)
    .bind(&failure_at)
    .execute(&state.pool)
    .await
    .expect("seed precise model route");
    let attempt_id = sqlx::query(
        "INSERT INTO pool_upstream_request_attempts (invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, started_at, status) VALUES ('precise-model-health', ?1, '/v1/responses', 'pool', 'gpt-precise', ?2, 'route', 1, 1, 0, ?3, 'failed')",
    )
    .bind(format_utc_iso(Utc::now()))
    .bind(account_id)
    .bind(&request_started_at)
    .execute(&state.pool)
    .await
    .expect("insert precise model attempt")
    .last_insert_rowid();

    record_pool_route_success_for_endpoint_with_image_intent_for_attempt(
        &state.pool,
        account_id,
        request_started_at_utc,
        None,
        None,
        "/v1/responses",
        ImageIntent::Unknown,
        Some(attempt_id),
    )
    .await
    .expect("record same-second model success");

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load precise model route")
        .into_iter()
        .find(|route| route.model == "gpt-precise")
        .expect("precise model route exists");
    assert_eq!(route.state, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(route.failure_count, 0);
}

#[tokio::test]
async fn manual_model_route_reset_fences_in_flight_failure() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Reset fence model health",
        "reset-fence-model-health-key",
        None,
        Some("https://reset-fence-model-health.example.com/backend-api/codex"),
    )
    .await;
    observe_model_route_seen(&state.pool, account_id, Some("gpt-reset-fence"))
        .await
        .expect("observe reset fence model");
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET cache_concurrency_limit = 1, cache_usage_missing_since = ?2, cache_usage_missing_reason = 'missing_input_tokens' WHERE account_id = ?1 AND model = 'gpt-reset-fence'",
    )
    .bind(account_id)
    .bind(format_utc_iso(Utc::now()))
    .execute(&state.pool)
    .await
    .expect("seed resettable cache-usage marker");

    let attempt_started_at = format_utc_iso(Utc::now() - chrono::Duration::seconds(2));
    let attempt_id = sqlx::query(
        "INSERT INTO pool_upstream_request_attempts (invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, started_at, status) VALUES ('reset-fence-model-health', ?1, '/v1/responses', 'pool', 'gpt-reset-fence', ?2, 'route', 1, 1, 0, ?3, 'failed')",
    )
    .bind(format_utc_iso(Utc::now()))
    .bind(account_id)
    .bind(&attempt_started_at)
    .execute(&state.pool)
    .await
    .expect("insert reset fence model attempt")
    .last_insert_rowid();

    let traffic_before_reset = sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT last_seen_at, last_success_at FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = 'gpt-reset-fence'",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load reset fence timestamps before reset");

    reset_model_route(&state.pool, account_id, "gpt-reset-fence")
        .await
        .expect("reset model route")
        .expect("reset model route exists");
    record_model_route_failure_from_attempt(
        &state.pool,
        account_id,
        attempt_id,
        StatusCode::BAD_REQUEST,
        Some("model unavailable"),
        Some("model_unavailable"),
    )
    .await
    .expect("record in-flight model failure");

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load reset fence route")
        .into_iter()
        .find(|item| item.model == "gpt-reset-fence")
        .expect("reset fence route exists");
    assert_eq!(route.state, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(route.priority, MODEL_ROUTE_PRIORITY_NORMAL);
    assert_eq!(route.failure_count, 0);
    assert!(route.last_failure_at.is_none());
    assert!(route.cache_usage_missing_since.is_none());
    assert!(route.cache_usage_missing_reason.is_none());

    let traffic_after_reset = sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT last_seen_at, last_success_at FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = 'gpt-reset-fence'",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load reset fence timestamps after reset");
    assert_eq!(traffic_after_reset, traffic_before_reset);
}

#[tokio::test]
async fn api_key_model_route_health_isolated_and_resettable() {
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
async fn concurrent_model_failures_reach_cooldown_threshold_without_lost_updates() {
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
async fn sparse_model_failures_keep_count_until_cooldown_threshold() {
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
async fn expired_model_cooldown_reports_transition_at_expiry() {
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
async fn current_quota_route_failure_survives_informational_account_updates() {
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
async fn oauth_summary_exports_missing_refresh_token_flag() {
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
fn pool_blocked_failure_kinds_are_not_temporary_route_failures() {
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
async fn record_pool_route_success_does_not_clear_newer_route_failure_state() {
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
async fn model_success_is_recorded_when_account_success_is_stale() {
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
async fn image_intent_route_success_learns_supported_capability() {
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
async fn chat_completions_route_learning_stays_on_chat_axis() {
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
async fn standalone_search_learning_is_api_key_only_and_route_specific() {
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
async fn standalone_search_override_round_trips_through_account_detail() {
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
async fn standalone_search_override_is_rejected_for_oauth_accounts() {
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
async fn image_intent_explicit_unsupported_failure_learns_unsupported_capability() {
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
async fn image_intent_validation_failure_does_not_learn_unsupported_capability() {
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
async fn mark_account_sync_success_preserves_route_cooldown_state() {
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
async fn mark_account_sync_success_clears_hard_unavailable_state_when_requested() {
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
async fn sync_api_key_account_preserves_route_cooldown_state() {
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
async fn sync_api_key_account_keeps_hard_unavailable_accounts_blocked() {
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
async fn sync_scope_reuses_live_reserved_node_for_same_account_before_shared_group_probe() {
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
async fn oauth_sync_refresh_due_reuses_sync_only_scope_for_token_refresh() {
    let (proxy_url, usage_requests, token_requests, server) =
        spawn_proxy_only_oauth_sync_server().await;
    let state = test_app_state_with_usage_and_oauth_base(
        "http://unreachable.invalid/backend-api",
        "http://unreachable.invalid",
    )
    .await;
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

    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load refresh-due account")
        .expect("refresh-due account exists");
    sync_oauth_account(state.as_ref(), &row, SyncCause::Manual)
        .await
        .expect("refresh-due sync should reuse the sync-only scoped node for refresh");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load refresh-due account after sync")
        .expect("refresh-due account still exists");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_error.is_none());
    assert!(after.last_route_failure_kind.is_none());
    assert!(after.last_successful_sync_at.is_some());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED)
    );
    assert_eq!(token_requests.load(Ordering::SeqCst), 1);
    assert_eq!(usage_requests.load(Ordering::SeqCst), 1);

    let decrypted = decrypt_credentials(
        crypto_key,
        after
            .encrypted_credentials
            .as_deref()
            .expect("encrypted oauth credentials"),
    )
    .expect("decrypt refreshed credentials");
    let StoredCredentials::Oauth(credentials) = decrypted else {
        panic!("unexpected credential kind after refresh-due sync")
    };
    assert_eq!(credentials.access_token, "proxy-refreshed-access-token");
    assert_eq!(
        credentials.refresh_token.as_deref(),
        Some("proxy-refreshed-refresh-token")
    );

    server.abort();
}

#[tokio::test]
async fn sync_scope_falls_back_to_shared_bound_group_when_exclusive_slot_is_full() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
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

    seed_hard_unavailable_route_failure(
        &state.pool,
        queued_account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    let assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
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
            .contains_key(&queued_account_id),
        "queued account should remain unassigned when the only slot is occupied",
    );

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
async fn manual_sync_allows_group_node_shunt_unassigned_account_to_probe_bound_node() {
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

    seed_hard_unavailable_route_failure(
        &state.pool,
        queued_account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    let assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
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
            .contains_key(&queued_account_id),
        "queued account should remain unassigned when the only slot is occupied",
    );

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

#[tokio::test]
async fn maintenance_sync_allows_group_node_shunt_unassigned_account_to_probe_bound_node() {
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
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let occupying_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Occupying Maintenance OAuth",
        "occupying-maintenance@example.com",
        "org_occupying_maintenance",
        "user_occupying_maintenance",
    )
    .await;
    let queued_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Queued Maintenance OAuth",
        "queued-maintenance@example.com",
        "org_queued_maintenance",
        "user_queued_maintenance",
    )
    .await;

    set_test_account_group_name(&state.pool, occupying_account_id, Some("node-shunt-maint")).await;
    set_test_account_group_name(&state.pool, queued_account_id, Some("node-shunt-maint")).await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-maint",
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
    .expect("save node shunt maintenance metadata");
    drop(conn);

    seed_hard_unavailable_route_failure(
        &state.pool,
        queued_account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    let outcome = state
        .upstream_accounts
        .account_ops
        .run_maintenance_sync(state.clone(), queued_account_id)
        .await
        .expect("maintenance sync should execute via shared bound-node probe");
    assert!(matches!(outcome, MaintenanceDispatchOutcome::Executed));

    let after = load_upstream_account_row(&state.pool, queued_account_id)
        .await
        .expect("load queued maintenance account after sync")
        .expect("queued maintenance account still exists");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_error.is_none());
    assert!(after.last_route_failure_kind.is_none());
    assert!(after.last_successful_sync_at.is_some());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED)
    );
    let detail = load_upstream_account_detail_with_actual_usage(state.as_ref(), queued_account_id)
        .await
        .expect("load queued maintenance detail")
        .expect("queued maintenance detail exists");
    assert_eq!(
        detail.summary.routing_block_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_GROUP_NODE_SHUNT_UNASSIGNED),
    );

    server.abort();
}

#[tokio::test]
async fn bulk_sync_allows_group_node_shunt_unassigned_account_to_probe_bound_node() {
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
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let occupying_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Occupying Bulk OAuth",
        "occupying-bulk@example.com",
        "org_occupying_bulk",
        "user_occupying_bulk",
    )
    .await;
    let queued_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Queued Bulk OAuth",
        "queued-bulk@example.com",
        "org_queued_bulk",
        "user_queued_bulk",
    )
    .await;

    set_test_account_group_name(&state.pool, occupying_account_id, Some("node-shunt-bulk")).await;
    set_test_account_group_name(&state.pool, queued_account_id, Some("node-shunt-bulk")).await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-bulk",
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
    .expect("save node shunt bulk metadata");
    drop(conn);

    seed_hard_unavailable_route_failure(
        &state.pool,
        queued_account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    let response = create_bulk_upstream_account_sync_job(
        State(state.clone()),
        HeaderMap::new(),
        Json(BulkUpstreamAccountSyncJobRequest {
            account_ids: vec![queued_account_id],
        }),
    )
    .await
    .expect("create bulk sync job")
    .0;
    let job = state
        .upstream_accounts
        .get_bulk_sync_job(&response.job_id)
        .await
        .expect("bulk sync job exists");
    let terminal = timeout(Duration::from_secs(15), async {
        loop {
            if let Some(terminal) = job.terminal_event.lock().await.clone() {
                return terminal;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("bulk sync job should finish within timeout");
    let BulkUpstreamAccountSyncTerminalEvent::Completed(payload) = terminal else {
        panic!("bulk sync job should complete successfully");
    };
    assert_eq!(payload.counts.total, 1);
    assert_eq!(payload.counts.completed, 1);
    assert_eq!(payload.counts.failed, 0);
    assert_eq!(payload.snapshot.rows.len(), 1);
    assert_eq!(
        payload.snapshot.rows[0].status,
        BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_SUCCEEDED
    );
    assert_eq!(payload.snapshot.rows[0].account_id, queued_account_id);

    let after = load_upstream_account_row(&state.pool, queued_account_id)
        .await
        .expect("load queued bulk account after sync")
        .expect("queued bulk account still exists");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_successful_sync_at.is_some());

    server.abort();
}

#[tokio::test]
async fn detail_preserves_group_node_shunt_unassigned_routing_block_reason() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
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

    let detail = load_upstream_account_detail_with_actual_usage(state.as_ref(), queued_account_id)
        .await
        .expect("load queued account detail")
        .expect("queued account detail exists");
    assert_eq!(detail.summary.id, queued_account_id);
    assert_eq!(
        detail.summary.routing_block_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_GROUP_NODE_SHUNT_UNASSIGNED),
    );
    assert_eq!(
        detail.summary.routing_block_reason_message.as_deref(),
        Some(group_node_shunt_unassigned_error_message()),
    );
}

#[tokio::test]
async fn list_upstream_accounts_applies_node_shunt_idle_rewrite_before_filters() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let occupying_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Occupying Filtered OAuth",
        "occupying-filtered@example.com",
        "org_occupying_filtered",
        "user_occupying_filtered",
    )
    .await;
    let queued_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Queued Filtered OAuth",
        "queued-filtered@example.com",
        "org_queued_filtered",
        "user_queued_filtered",
    )
    .await;

    set_test_account_group_name(&state.pool, occupying_account_id, Some("node-shunt-filter")).await;
    set_test_account_group_name(&state.pool, queued_account_id, Some("node-shunt-filter")).await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-filter",
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
    .expect("save node shunt filter metadata");
    drop(conn);

    let occupying_selected_at = format_utc_iso(Utc::now() - ChronoDuration::minutes(3));
    let queued_selected_at = format_utc_iso(Utc::now() - ChronoDuration::minutes(1));
    for (account_id, selected_at) in [
        (occupying_account_id, occupying_selected_at),
        (queued_account_id, queued_selected_at),
    ] {
        sqlx::query(
            r#"
                UPDATE pool_upstream_accounts
                SET last_selected_at = ?2,
                    updated_at = ?2
                WHERE id = ?1
                "#,
        )
        .bind(account_id)
        .bind(selected_at)
        .execute(&state.pool)
        .await
        .expect("seed last_selected_at");
    }

    let assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
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
            .contains_key(&queued_account_id),
        "queued account should be unassigned before list filtering",
    );

    let mut all_items = load_upstream_account_summaries_for_query(
        &state.pool,
        &state.config,
        &ListUpstreamAccountsQuery::default(),
    )
    .await
    .expect("load upstream account summaries");
    enrich_node_shunt_routing_block_reasons(state.as_ref(), &mut all_items)
        .await
        .expect("enrich node shunt routing block reasons");

    let idle_filters = normalize_upstream_account_list_filters(&ListUpstreamAccountsQuery {
        work_status: vec![UPSTREAM_ACCOUNT_WORK_STATUS_IDLE.to_string()],
        ..ListUpstreamAccountsQuery::default()
    });
    let idle_items = filter_upstream_account_summaries(all_items.clone(), &idle_filters);
    let idle_metrics = build_upstream_account_list_metrics(&idle_items);

    assert_eq!(idle_items.len(), 1);
    assert_eq!(idle_metrics.total, 1);
    assert_eq!(idle_items[0].id, queued_account_id);
    assert_eq!(idle_items[0].work_status, UPSTREAM_ACCOUNT_WORK_STATUS_IDLE);
    assert_eq!(
        idle_items[0].routing_block_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_GROUP_NODE_SHUNT_UNASSIGNED),
    );

    let working_filters = normalize_upstream_account_list_filters(&ListUpstreamAccountsQuery {
        work_status: vec![UPSTREAM_ACCOUNT_WORK_STATUS_WORKING.to_string()],
        ..ListUpstreamAccountsQuery::default()
    });
    let working_items = filter_upstream_account_summaries(all_items, &working_filters);
    assert_eq!(working_items.len(), 1);
    assert_eq!(working_items[0].id, occupying_account_id);
    assert!(
        working_items
            .iter()
            .all(|item| item.id != queued_account_id),
        "queued account should not remain in working results after node shunt rewrite",
    )
}

#[tokio::test]
async fn sync_api_key_account_clears_stale_manual_recovery_marker_on_active_rows() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Active API Key With Stale Marker").await;
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
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
        SyncSuccessRouteState::PreserveFailureState,
    )
    .await
    .expect("mark legacy sync success");
    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load api key row")
        .expect("api key row exists");
    assert_eq!(row.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(
        row.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );

    sync_api_key_account(&pool, &row, SyncCause::Maintenance)
        .await
        .expect("sync api key account");
    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after api key sync")
        .expect("row exists after api key sync");

    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_route_failure_kind.is_none());
    assert!(after.cooldown_until.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED)
    );
}

#[tokio::test]
async fn updating_api_key_reactivates_manually_recoverable_account() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Recoverable API Key",
        "recoverable-api-key",
        None,
        Some("https://recoverable-api-key.example.com/backend-api/codex"),
    )
    .await;
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    let mut availability = state.pool_routing_availability.subscribe();

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
                note: None,
                group_note: None,
                concurrency_limit: None,
                upstream_base_url: OptionalField::Missing,
                bound_proxy_keys: OptionalField::Missing,
                enabled: None,
                is_mother: None,
                api_key: Some("sk-live-new".to_string()),
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: None,
                routing_rule: None,
                ..UpdateUpstreamAccountRequest::default()
            },
        )
        .await
        .expect("update api key account");
    tokio::time::timeout(Duration::from_secs(1), availability.changed())
        .await
        .expect("manual recovery should publish routing availability")
        .expect("routing availability signal should stay open");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load row after api key update")
        .expect("row exists after api key update");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_error.is_none());
    assert!(after.last_route_failure_kind.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_ACCOUNT_UPDATED)
    );
}

#[tokio::test]
async fn successful_account_sync_publishes_routing_availability_on_recovery() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Sync Recovery Signal",
        "sync-recovery-signal-key",
        None,
        Some("https://sync-recovery-signal.example.com/backend-api/codex"),
    )
    .await;
    set_account_status(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        Some("temporary sync failure"),
    )
    .await
    .expect("make account unavailable before sync");
    let mut availability = state.pool_routing_availability.subscribe();

    sync_upstream_account_by_id(state.as_ref(), account_id, SyncCause::Manual)
        .await
        .expect("sync should recover the account");
    tokio::time::timeout(Duration::from_secs(1), availability.changed())
        .await
        .expect("sync recovery should publish routing availability")
        .expect("routing availability signal should stay open");

    let recovered = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load recovered account")
        .expect("recovered account exists");
    assert_eq!(recovered.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(is_account_selectable_for_fresh_assignment(
        &recovered,
        false,
        Utc::now()
    ));
}

#[tokio::test]
async fn imported_oauth_probe_publishes_routing_availability_on_recovery() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Imported OAuth Recovery Signal",
        "import-recovery@example.com",
        "org_import_recovery",
        "user_import_recovery",
    )
    .await;
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    let mut availability = state.pool_routing_availability.subscribe();
    let probe = ImportedOauthProbeOutcome {
        token_expires_at: format_utc_iso(Utc::now() + ChronoDuration::days(30)),
        credentials: StoredOauthCredentials {
            access_token: "imported-access-token".to_string(),
            refresh_token: Some("imported-refresh-token".to_string()),
            id_token: test_id_token(
                "import-recovery@example.com",
                Some("org_import_recovery"),
                Some("user_import_recovery"),
                Some("team"),
            ),
            token_type: Some("Bearer".to_string()),
        },
        claims: ChatgptJwtClaims::default(),
        usage_snapshot: Some(NormalizedUsageSnapshot {
            plan_type: Some("team".to_string()),
            limit_id: "codex".to_string(),
            limit_name: Some("Codex".to_string()),
            primary: None,
            secondary: None,
            credits: None,
        }),
        maintenance_proxy_snapshot: None,
        exhausted: false,
        usage_snapshot_warning: None,
    };

    apply_imported_oauth_probe_result(state.as_ref(), account_id, &probe)
        .await
        .expect("apply imported OAuth recovery probe");
    tokio::time::timeout(Duration::from_secs(1), availability.changed())
        .await
        .expect("OAuth import recovery should publish routing availability")
        .expect("routing availability signal should stay open");

    let recovered = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load imported OAuth account")
        .expect("imported OAuth account exists");
    assert!(is_account_selectable_for_fresh_assignment(
        &recovered,
        false,
        Utc::now()
    ));

    let unchanged_availability = state.pool_routing_availability.subscribe();
    apply_imported_oauth_probe_result(state.as_ref(), account_id, &probe)
        .await
        .expect("reapply imported OAuth probe to an already routable account");
    assert!(
        !unchanged_availability
            .has_changed()
            .expect("read availability"),
        "an already routable account must not publish a redundant availability event"
    );
}

#[tokio::test]
async fn oauth_sync_keeps_quota_exhausted_accounts_blocked_until_snapshot_recovers() {
    let (base_url, server) = spawn_usage_snapshot_server(
        StatusCode::OK,
        json!({
            "planType": "team",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 100,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                }
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Exhausted OAuth",
        "exhausted@example.com",
        "org_exhausted",
        "user_exhausted",
    )
    .await;
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");

    sync_oauth_account(&state, &row, SyncCause::Manual)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after sync")
        .expect("oauth row exists after sync");
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
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    server.abort();
}

#[tokio::test]
async fn oauth_sync_ignores_stale_input_row_after_newer_quota_hard_stop() {
    let (base_url, server) = spawn_usage_snapshot_server(
        StatusCode::OK,
        json!({
            "planType": "team",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 100,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                }
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Stale OAuth Input Row",
        "stale-input@example.com",
        "org_stale_input",
        "user_stale_input",
    )
    .await;
    let stale_row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    sync_oauth_account(&state, &stale_row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after stale sync")
        .expect("oauth row exists after stale sync");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_RECOVERY_BLOCKED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    server.abort();
}

#[tokio::test]
async fn oauth_sync_demotes_active_stale_quota_marker_when_snapshot_is_still_exhausted() {
    let (base_url, server) = spawn_usage_snapshot_server(
        StatusCode::OK,
        json!({
            "planType": "team",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 100,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                }
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Legacy Active Exhausted OAuth",
        "legacy-exhausted@example.com",
        "org_legacy_exhausted",
        "user_legacy_exhausted",
    )
    .await;
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    mark_account_sync_success(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
        SyncSuccessRouteState::PreserveFailureState,
    )
    .await
    .expect("mark legacy sync success");
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");
    assert_eq!(row.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(
        row.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );

    sync_oauth_account(&state, &row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after sync")
        .expect("oauth row exists after sync");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_RECOVERY_BLOCKED)
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    server.abort();
}

#[tokio::test]
async fn oauth_sync_reactivates_quota_exhausted_account_once_snapshot_recovers() {
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
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Recovered OAuth Sync",
        "recovered@example.com",
        "org_recovered",
        "user_recovered",
    )
    .await;
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");

    sync_oauth_account(&state, &row, SyncCause::Manual)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after recovery")
        .expect("oauth row exists after recovery");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_error.is_none());
    assert!(after.last_route_failure_kind.is_none());
    assert!(after.last_successful_sync_at.is_some());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED)
    );
    server.abort();
}

#[tokio::test]
async fn oauth_sync_retry_after_refresh_settles_to_needs_reauth_without_stale_syncing() {
    let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
            spawn_sequenced_oauth_sync_server(
                vec![
                    (
                        StatusCode::UNAUTHORIZED,
                        json!({
                            "error": {
                                "message": "Session cookie expired during usage snapshot"
                            }
                        }),
                    ),
                    (
                        StatusCode::FORBIDDEN,
                        json!({
                            "error": {
                                "message": "Authentication token has been invalidated, please sign in again"
                            }
                        }),
                    ),
                ],
                json!({
                    "access_token": "refreshed-access-token",
                    "refresh_token": "refresh-token-rotated",
                    "id_token": test_id_token(
                        "reauth-required@example.com",
                        Some("org_retry_reauth"),
                        Some("user_retry_reauth"),
                        Some("team"),
                    ),
                    "token_type": "Bearer",
                    "expires_in": 3600
                }),
            )
            .await;
    let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Retry Needs Reauth OAuth",
        "reauth-required@example.com",
        "org_retry_reauth",
        "user_retry_reauth",
    )
    .await;
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");

    sync_oauth_account(&state, &row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after retry failure")
        .expect("oauth row exists after retry failure");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
    assert!(after.last_synced_at.is_some());
    assert!(after.last_successful_sync_at.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED)
    );
    assert_eq!(after.last_action_http_status, Some(403));
    assert_eq!(
        after.last_error.as_deref(),
        Some(
            "usage endpoint returned 403 Forbidden: Authentication token has been invalidated, please sign in again"
        )
    );
    assert!(after.last_action_at.is_some());

    let decrypted = decrypt_credentials(
        crypto_key,
        after
            .encrypted_credentials
            .as_deref()
            .expect("encrypted oauth credentials"),
    )
    .expect("decrypt refreshed credentials");
    let StoredCredentials::Oauth(credentials) = decrypted else {
        panic!("unexpected credential kind after refresh")
    };
    assert_eq!(credentials.access_token, "refreshed-access-token");
    assert_eq!(
        credentials.refresh_token.as_deref(),
        Some("refresh-token-rotated")
    );

    let summary = build_summary_from_row(
        &after,
        None,
        after.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
    assert_eq!(
        summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
    );
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);

    let detail = load_upstream_account_detail(&state.pool, account_id)
        .await
        .expect("load detail export")
        .expect("detail export exists");
    assert_eq!(
        detail.summary.display_status,
        UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH
    );
    assert_eq!(
        detail.summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
    );
    assert_eq!(detail.summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
    assert_eq!(
        detail
            .recent_actions
            .first()
            .map(|event| event.action.as_str()),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        detail
            .recent_actions
            .first()
            .and_then(|event| event.reason_code.as_deref()),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED)
    );
    assert_eq!(usage_requests.load(Ordering::SeqCst), 2);
    assert_eq!(token_requests.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
async fn oauth_sync_retry_after_refresh_records_non_auth_terminal_failure_without_stale_syncing() {
    let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
        spawn_sequenced_oauth_sync_server(
            vec![
                (
                    StatusCode::UNAUTHORIZED,
                    json!({
                        "error": {
                            "message": "Session cookie expired during usage snapshot"
                        }
                    }),
                ),
                (
                    StatusCode::BAD_GATEWAY,
                    json!({
                        "error": {
                            "message": "gateway temporarily unavailable"
                        }
                    }),
                ),
            ],
            json!({
                "access_token": "refreshed-temporary-token",
                "refresh_token": "refresh-token-rotated",
                "id_token": test_id_token(
                    "transport-failure@example.com",
                    Some("org_retry_gateway"),
                    Some("user_retry_gateway"),
                    Some("team"),
                ),
                "token_type": "Bearer",
                "expires_in": 3600
            }),
        )
        .await;
    let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Retry Gateway Failure OAuth",
        "transport-failure@example.com",
        "org_retry_gateway",
        "user_retry_gateway",
    )
    .await;
    seed_route_cooldown(
        &state.pool,
        account_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        300,
    )
    .await;
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");

    sync_oauth_account(&state, &row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after gateway failure")
        .expect("oauth row exists after gateway failure");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_synced_at.is_some());
    assert!(after.last_successful_sync_at.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some("upstream_http_5xx")
    );
    assert_eq!(after.last_action_http_status, Some(502));
    assert_eq!(
        after.last_error.as_deref(),
        Some("usage endpoint returned 502 Bad Gateway: gateway temporarily unavailable")
    );
    assert!(after.last_action_at.is_some());
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    );
    assert!(after.cooldown_until.is_some());
    assert_eq!(after.consecutive_route_failures, 1);

    let summary = build_summary_from_row(
        &after,
        None,
        after.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.work_status, UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED);
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);

    let detail = load_upstream_account_detail(&state.pool, account_id)
        .await
        .expect("load detail export")
        .expect("detail export exists");
    assert_eq!(
        detail.summary.display_status,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE
    );
    assert_eq!(
        detail.summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED
    );
    assert_eq!(detail.summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
    assert_eq!(
        detail
            .recent_actions
            .first()
            .map(|event| event.action.as_str()),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        detail
            .recent_actions
            .first()
            .and_then(|event| event.http_status),
        Some(502)
    );
    assert_eq!(usage_requests.load(Ordering::SeqCst), 2);
    assert_eq!(token_requests.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
async fn oauth_sync_retry_after_refresh_preserves_quota_marker_from_current_db_state() {
    let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
        spawn_sequenced_oauth_sync_server(
            vec![
                (
                    StatusCode::UNAUTHORIZED,
                    json!({
                        "error": {
                            "message": "Session cookie expired during usage snapshot"
                        }
                    }),
                ),
                (
                    StatusCode::BAD_GATEWAY,
                    json!({
                        "error": {
                            "message": "gateway temporarily unavailable"
                        }
                    }),
                ),
            ],
            json!({
                "access_token": "refreshed-quota-preserving-token",
                "refresh_token": "refresh-token-rotated",
                "id_token": test_id_token(
                    "retry-quota-preserve@example.com",
                    Some("org_retry_quota_preserve"),
                    Some("user_retry_quota_preserve"),
                    Some("team"),
                ),
                "token_type": "Bearer",
                "expires_in": 3600
            }),
        )
        .await;
    let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Retry Gateway Quota Preserve OAuth",
        "retry-quota-preserve@example.com",
        "org_retry_quota_preserve",
        "user_retry_quota_preserve",
    )
    .await;
    let stale_row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    sync_oauth_account(&state, &stale_row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after gateway failure")
        .expect("oauth row exists after gateway failure");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_synced_at.is_some());
    assert!(after.last_successful_sync_at.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some("upstream_http_5xx")
    );
    assert_eq!(after.last_action_http_status, Some(502));
    assert_eq!(
        after.last_error.as_deref(),
        Some("usage endpoint returned 502 Bad Gateway: gateway temporarily unavailable")
    );
    assert!(after.last_action_at.is_some());
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    assert_eq!(after.last_route_failure_at, after.last_error_at);

    let summary = build_summary_from_row(
        &after,
        None,
        after.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(
        summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);

    let detail = load_upstream_account_detail(&state.pool, account_id)
        .await
        .expect("load detail export")
        .expect("detail export exists");
    assert_eq!(
        detail.summary.display_status,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE
    );
    assert_eq!(
        detail.summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(detail.summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);

    assert_eq!(usage_requests.load(Ordering::SeqCst), 2);
    assert_eq!(token_requests.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
async fn oauth_sync_refresh_failure_preserves_quota_marker_from_current_db_state() {
    let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
        spawn_sequenced_oauth_sync_server(
            vec![(
                StatusCode::UNAUTHORIZED,
                json!({
                    "error": {
                        "message": "Session cookie expired during usage snapshot"
                    }
                }),
            )],
            json!({
                "unexpected": "shape"
            }),
        )
        .await;
    let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Refresh Failure Quota Preserve OAuth",
        "refresh-quota-preserve@example.com",
        "org_refresh_quota_preserve",
        "user_refresh_quota_preserve",
    )
    .await;
    let stale_row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    sync_oauth_account(&state, &stale_row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after refresh failure")
        .expect("oauth row exists after refresh failure");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR)
    );
    assert_eq!(after.last_action_http_status, None);
    assert!(
        after
            .last_error
            .as_deref()
            .is_some_and(|message| message.contains("failed to decode OAuth token response"))
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    assert_eq!(after.last_route_failure_at, after.last_error_at);

    let summary = build_summary_from_row(
        &after,
        None,
        after.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(
        summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);

    let detail = load_upstream_account_detail(&state.pool, account_id)
        .await
        .expect("load detail export")
        .expect("detail export exists");
    assert_eq!(
        detail.summary.display_status,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE
    );
    assert_eq!(
        detail.summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(detail.summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);

    assert_eq!(usage_requests.load(Ordering::SeqCst), 1);
    assert_eq!(token_requests.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
async fn oauth_sync_direct_fetch_failure_preserves_quota_marker_from_current_db_state() {
    let (usage_base_url, server) = spawn_usage_snapshot_server(
        StatusCode::BAD_GATEWAY,
        json!({
            "error": {
                "message": "gateway temporarily unavailable"
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&usage_base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Direct Failure Quota Preserve OAuth",
        "direct-quota-preserve@example.com",
        "org_direct_quota_preserve",
        "user_direct_quota_preserve",
    )
    .await;
    let stale_row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    sync_oauth_account(&state, &stale_row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after direct fetch failure")
        .expect("oauth row exists after direct fetch failure");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some("upstream_http_5xx")
    );
    assert_eq!(after.last_action_http_status, Some(502));
    assert!(
        after
            .last_error
            .as_deref()
            .is_some_and(|message| message.contains("502 Bad Gateway"))
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    assert_eq!(after.last_route_failure_at, after.last_error_at);

    let summary = build_summary_from_row(
        &after,
        None,
        after.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(
        summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);

    let detail = load_upstream_account_detail(&state.pool, account_id)
        .await
        .expect("load detail export")
        .expect("detail export exists");
    assert_eq!(
        detail.summary.display_status,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE
    );
    assert_eq!(
        detail.summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(detail.summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
    server.abort();
}

#[tokio::test]
async fn classified_sync_failure_preserves_existing_route_cooldown_across_new_error_timestamp() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Preserved Cooldown OAuth").await;
    let previous_failure_at = format_utc_iso(Utc::now() - ChronoDuration::minutes(2));
    let cooldown_until = format_utc_iso(Utc::now() + ChronoDuration::minutes(5));

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
                last_action = ?7,
                last_action_source = ?8,
                last_action_reason_code = ?9,
                last_action_reason_message = ?3,
                last_action_http_status = ?10,
                last_action_at = ?4,
                updated_at = ?4
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind("seed preserved cooldown")
    .bind(&previous_failure_at)
    .bind(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    .bind(&cooldown_until)
    .bind(UPSTREAM_ACCOUNT_ACTION_ROUTE_COOLDOWN_STARTED)
    .bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL)
    .bind(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT)
    .bind(429)
    .execute(&pool)
    .await
    .expect("seed preserved cooldown row");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load seeded cooldown row")
        .expect("seeded cooldown row exists");
    record_classified_account_sync_failure(
        &pool,
        &row,
        row.status.as_str(),
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
        "usage endpoint returned 502 Bad Gateway: gateway temporarily unavailable",
    )
    .await
    .expect("record classified retry failure");

    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load cooldown row after retry failure")
        .expect("cooldown row after retry failure exists");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some("upstream_http_5xx")
    );
    assert_eq!(after.last_action_http_status, Some(502));
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    );
    assert_eq!(after.last_route_failure_at, after.last_error_at);
    assert_ne!(
        after.last_route_failure_at.as_deref(),
        Some(previous_failure_at.as_str())
    );

    let summary = build_summary_from_row(
        &after,
        None,
        after.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.work_status, UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED);
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
}

#[tokio::test]
async fn classified_sync_hard_unavailable_replaces_stale_quota_marker_from_current_syncing_row() {
    let pool = test_pool().await;

    for (reason_code, http_status, failure_kind, error_message) in [
        (
            "upstream_http_401",
            StatusCode::UNAUTHORIZED,
            PROXY_FAILURE_UPSTREAM_HTTP_AUTH,
            "usage endpoint returned 401 Unauthorized: Missing scopes: api.responses.write",
        ),
        (
            "upstream_http_402",
            StatusCode::PAYMENT_REQUIRED,
            PROXY_FAILURE_UPSTREAM_HTTP_402,
            "usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}",
        ),
        (
            "upstream_http_403",
            StatusCode::FORBIDDEN,
            PROXY_FAILURE_UPSTREAM_HTTP_AUTH,
            "usage endpoint returned 403 Forbidden: You have insufficient permissions for this operation.",
        ),
    ] {
        let account_id =
            insert_oauth_account(&pool, &format!("Syncing hard unavailable {reason_code}")).await;

        seed_hard_unavailable_route_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            Some(429),
        )
        .await;
        set_account_status(&pool, account_id, UPSTREAM_ACCOUNT_STATUS_SYNCING, None)
            .await
            .expect("mark row syncing");

        let current_row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load current syncing row")
            .expect("current syncing row exists");
        assert_eq!(current_row.status, UPSTREAM_ACCOUNT_STATUS_SYNCING);

        record_classified_account_sync_failure(
            &pool,
            &current_row,
            UPSTREAM_ACCOUNT_STATUS_ACTIVE,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            error_message,
        )
        .await
        .expect("record hard unavailable failure against syncing row");

        let after = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load syncing row after hard unavailable failure")
            .expect("syncing row after hard unavailable failure exists");
        assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
        assert_eq!(after.last_action_reason_code.as_deref(), Some(reason_code));
        assert_eq!(
            after.last_action_http_status,
            Some(http_status.as_u16() as i64)
        );
        assert_eq!(after.last_route_failure_kind.as_deref(), Some(failure_kind));
        assert_eq!(after.last_route_failure_at, after.last_error_at);
        if reason_code == "upstream_http_402" {
            let cooldown_until = after
                .cooldown_until
                .as_deref()
                .and_then(parse_rfc3339_utc)
                .expect("maintenance-triggered 402 should write explicit cooldown");
            let failed_at = after
                .last_action_at
                .as_deref()
                .and_then(parse_rfc3339_utc)
                .expect("maintenance-triggered 402 should record last_action_at");
            assert_eq!(
                cooldown_until - failed_at,
                ChronoDuration::seconds(
                    UPSTREAM_ACCOUNT_UPSTREAM_REJECTED_MAINTENANCE_COOLDOWN_SECS,
                )
            );
        } else {
            assert_eq!(after.cooldown_until, None);
        }
        assert_eq!(after.temporary_route_failure_streak_started_at, None);

        let summary = build_summary_from_row(
            &after,
            None,
            after.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );
        assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
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
        assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
    }
}

#[tokio::test]
async fn classified_sync_wrapped_upstream_rejected_permission_keeps_existing_cooldown_policy() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Wrapped upstream rejected cooldown").await;

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load fresh row")
        .expect("fresh row exists");
    record_classified_account_sync_failure(
        &pool,
        &row,
        row.status.as_str(),
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
        "oauth_upstream_rejected_request: pool upstream responded with 403: Forbidden",
    )
    .await
    .expect("record wrapped upstream rejected sync failure");

    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after wrapped upstream rejected sync failure")
        .expect("row after wrapped upstream rejected sync failure exists");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some("upstream_http_403")
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_AUTH)
    );
    assert_eq!(
        after.cooldown_until, None,
        "wrapped upstream auth errors should keep the old no-cooldown behavior"
    );
}

#[tokio::test]
async fn classified_sync_failure_emits_suppressed_event_when_reason_toggle_disabled() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Suppressed Sync 402").await;
    sqlx::query(
        "UPDATE pool_upstream_accounts SET policy_status_change_upstream_http_402 = 0 WHERE id = ?1",
    )
    .bind(account_id)
    .execute(&pool)
    .await
    .expect("disable sync 402 status change toggle");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load fresh row")
        .expect("fresh row exists");
    record_classified_account_sync_failure(
        &pool,
        &row,
        row.status.as_str(),
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
        "usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}",
    )
    .await
    .expect("record suppressed sync failure");

    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after suppressed sync failure")
        .expect("row after suppressed sync failure exists");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(after.last_error, None);
    assert_eq!(after.last_action, None);
    assert_eq!(after.last_route_failure_kind, None);
    assert_eq!(after.cooldown_until, None);

    let detail = load_upstream_account_detail(&pool, account_id)
        .await
        .expect("load suppressed sync detail")
        .expect("suppressed sync detail exists");
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
        UPSTREAM_ACCOUNT_WORK_STATUS_IDLE
    );
    let event = detail
        .recent_actions
        .first()
        .expect("suppressed sync event should be recorded");
    assert_eq!(
        event.action,
        UPSTREAM_ACCOUNT_ACTION_STATUS_CHANGE_SUPPRESSED
    );
    assert_eq!(
        event.source,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE
    );
    assert_eq!(
        event.reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_402)
    );
    assert_eq!(event.http_status, Some(402));
    assert_eq!(
        event.failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_402)
    );
    assert!(
        event
            .reason_message
            .as_deref()
            .is_some_and(|value| value.contains("402 Payment Required"))
    );
}

#[tokio::test]
async fn classified_sync_non_rejected_failure_clears_existing_maintenance_rejected_cooldown() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Rejected Cooldown Replaced").await;

    record_account_sync_hard_unavailable(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ACTIVE,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            "upstream_http_402",
            "usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}",
            PROXY_FAILURE_UPSTREAM_HTTP_402,
        )
        .await
        .expect("seed maintenance rejected cooldown");

    let before = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row before replacement failure")
        .expect("row exists before replacement failure");
    assert!(before.cooldown_until.is_some());

    record_classified_account_sync_failure(
            &pool,
            &before,
            before.status.as_str(),
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            "usage endpoint returned 403 Forbidden: You have insufficient permissions for this operation.",
        )
        .await
        .expect("record replacement sync failure");

    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after replacement failure")
        .expect("row exists after replacement failure");
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some("upstream_http_403")
    );
    assert_eq!(after.cooldown_until, None);
}

#[tokio::test]
async fn mark_account_sync_success_clears_explicit_maintenance_upstream_rejected_cooldown() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Rejected Cooldown Success").await;

    record_account_sync_hard_unavailable(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ACTIVE,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            "upstream_http_402",
            "usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}",
            PROXY_FAILURE_UPSTREAM_HTTP_402,
        )
        .await
        .expect("seed maintenance rejected cooldown");

    let before = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row before success")
        .expect("row exists before success");
    assert!(before.cooldown_until.is_some());

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
        .expect("load row after success")
        .expect("row exists after success");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.cooldown_until.is_none());
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_402),
        "preserve-failure success should keep the last route failure marker while clearing the explicit maintenance cooldown"
    );
}

#[tokio::test]
async fn classified_sync_failure_preserves_quota_marker_from_current_syncing_row() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Quota Syncing Preserve").await;

    seed_hard_unavailable_route_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    set_account_status(&pool, account_id, UPSTREAM_ACCOUNT_STATUS_SYNCING, None)
        .await
        .expect("mark row syncing");

    let current_row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load current syncing row")
        .expect("current syncing row exists");
    assert_eq!(current_row.status, UPSTREAM_ACCOUNT_STATUS_SYNCING);

    record_classified_account_sync_failure(
        &pool,
        &current_row,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
        "usage endpoint returned 502 Bad Gateway: gateway temporarily unavailable",
    )
    .await
    .expect("record retry failure against syncing row");

    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load syncing row after retry failure")
        .expect("syncing row after retry failure exists");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some("upstream_http_5xx")
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    assert_eq!(after.last_route_failure_at, after.last_error_at);

    let summary = build_summary_from_row(
        &after,
        None,
        after.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(
        summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
}

#[tokio::test]
async fn oauth_sync_proactively_quarantines_snapshot_exhausted_account_without_prior_route_failure()
{
    let (base_url, server) = spawn_usage_snapshot_server(
        StatusCode::OK,
        json!({
            "planType": "team",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 100,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                }
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Sync Snapshot Exhausted",
        "snapshot-exhausted@example.com",
        "org_snapshot_exhausted",
        "user_snapshot_exhausted",
    )
    .await;
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");

    sync_oauth_account(&state, &row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after proactive quarantine")
        .expect("oauth row exists after proactive quarantine");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert!(after.last_successful_sync_at.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_HARD_UNAVAILABLE)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_USAGE_SNAPSHOT_EXHAUSTED)
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_USAGE_SNAPSHOT_QUOTA_EXHAUSTED)
    );
    server.abort();
}

#[tokio::test]
async fn resolver_short_circuits_when_only_persisted_snapshot_exhausted_accounts_remain() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let first = insert_api_key_account(&state.pool, "Exhausted A").await;
    let second = insert_api_key_account(&state.pool, "Exhausted B").await;
    let third = insert_api_key_account(&state.pool, "Exhausted C").await;
    let now_iso = format_utc_iso(Utc::now());
    for account_id in [first, second, third] {
        insert_limit_sample_with_usage(&state.pool, account_id, &now_iso, Some(100.0), Some(40.0))
            .await;
    }

    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve pool account");
    assert!(matches!(resolution, PoolAccountResolution::RateLimited));
}

#[tokio::test]
async fn resolver_skips_persisted_snapshot_exhausted_account_before_routing() {
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
async fn resolver_reuses_sticky_snapshot_exhausted_account_until_conversation_gets_429() {
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
async fn resolver_preserves_sticky_record_but_rotates_after_auth_hard_failure() {
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
async fn resolver_prefers_primary_priority_before_normal_and_fallback() {
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
async fn resolver_proactively_hands_off_fallback_sticky_to_higher_priority_account() {
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

    let resolution =
        resolve_pool_account_for_request(&state, Some(sticky_key), &[], &HashSet::new())
            .await
            .expect("resolve fallback sticky handoff");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected higher priority account to receive the request");
    };
    assert_eq!(account.account_id, primary_account_id);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::FreshAssignment
    );
    assert_eq!(
        load_sticky_route(&state.pool, sticky_key)
            .await
            .expect("load sticky route before success")
            .map(|route| route.account_id),
        Some(fallback_account_id),
    );

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

#[tokio::test]
async fn resolver_keeps_fallback_sticky_when_no_higher_priority_candidate_exists() {
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
async fn resolver_preserves_same_tier_fallback_penalty_failover_during_proactive_handoff() {
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
async fn resolver_allows_fallback_failover_when_sticky_source_is_unusable() {
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
async fn resolver_keeps_higher_priority_soft_degraded_candidate_ahead_of_lower_priority_ready_account()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let slot_owner_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Node Shunt Slot Owner",
        "sk-soft-degrade-owner",
        Some("soft-degrade-priority"),
        Some("https://soft-degrade-owner.example.com/backend-api/codex"),
    )
    .await;
    let soft_degraded_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Node Shunt Soft Degraded",
        "sk-soft-degrade-target",
        Some("soft-degrade-priority"),
        Some("https://soft-degrade-target.example.com/backend-api/codex"),
    )
    .await;
    let fallback_ready_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Fallback Ready Candidate",
        "sk-soft-degrade-fallback",
        Some("soft-degrade-fallback"),
        Some("https://soft-degrade-fallback.example.com/backend-api/codex"),
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

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "soft-degrade-priority",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: vec![FORWARD_PROXY_DIRECT_KEY.to_string()],
            node_shunt_enabled: true,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save node shunt metadata");
    save_group_metadata_record_conn(
        &mut conn,
        "soft-degrade-fallback",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: vec![FORWARD_PROXY_DIRECT_KEY.to_string()],
            node_shunt_enabled: false,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save fallback metadata");
    drop(conn);

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

#[test]
fn retry_original_node_candidates_sort_after_sendable_candidates_even_when_priority_is_higher() {
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

#[test]
fn overflow_candidates_sort_after_primary_candidates_even_when_priority_is_higher() {
    let overflow = PoolRoutingCandidateScore {
        eligibility: PoolRoutingCandidateEligibility::Assignable,
        route_binding_failure_penalty: 0,
        model_route_penalty: 0,
        routing_priority_rank: 0,
        capacity_lane: PoolRoutingCandidateCapacityLane::Overflow,
        dispatch_state: PoolRoutingCandidateDispatchState::ReadyOnOwnedNode,
        single_account_rotation_enabled: false,
        secondary_reset_proximity_secs: None,
        primary_reset_proximity_secs: None,
        scarcity_score: 0.0,
        effective_load: 9,
        last_selected_at: None,
        account_id: 12,
    };
    let primary = PoolRoutingCandidateScore {
        eligibility: PoolRoutingCandidateEligibility::Assignable,
        route_binding_failure_penalty: 0,
        model_route_penalty: 0,
        routing_priority_rank: 2,
        capacity_lane: PoolRoutingCandidateCapacityLane::Primary,
        dispatch_state: PoolRoutingCandidateDispatchState::ReadyOnOwnedNode,
        single_account_rotation_enabled: false,
        secondary_reset_proximity_secs: None,
        primary_reset_proximity_secs: None,
        scarcity_score: 0.0,
        effective_load: 1,
        last_selected_at: None,
        account_id: 13,
    };

    assert_eq!(
        compare_pool_routing_candidate_scores(&overflow, &primary),
        std::cmp::Ordering::Greater
    );
    assert_eq!(
        compare_pool_routing_candidate_scores(&primary, &overflow),
        std::cmp::Ordering::Less
    );
}

#[test]
fn reset_proximity_sorts_before_usage_pressure_after_priority_gates() {
    let closer_secondary_reset = PoolRoutingCandidateScore {
        eligibility: PoolRoutingCandidateEligibility::Assignable,
        route_binding_failure_penalty: 0,
        model_route_penalty: 0,
        routing_priority_rank: 1,
        capacity_lane: PoolRoutingCandidateCapacityLane::Primary,
        dispatch_state: PoolRoutingCandidateDispatchState::ReadyOnOwnedNode,
        single_account_rotation_enabled: true,
        secondary_reset_proximity_secs: Some(60),
        primary_reset_proximity_secs: Some(60 * 60 * 4),
        scarcity_score: 0.95,
        effective_load: 2,
        last_selected_at: Some("2026-03-23T12:00:00Z".to_string()),
        account_id: 20,
    };
    let farther_secondary_reset = PoolRoutingCandidateScore {
        eligibility: PoolRoutingCandidateEligibility::Assignable,
        route_binding_failure_penalty: 0,
        model_route_penalty: 0,
        routing_priority_rank: 1,
        capacity_lane: PoolRoutingCandidateCapacityLane::Primary,
        dispatch_state: PoolRoutingCandidateDispatchState::ReadyOnOwnedNode,
        single_account_rotation_enabled: true,
        secondary_reset_proximity_secs: Some(60 * 60 * 24),
        primary_reset_proximity_secs: Some(60),
        scarcity_score: 0.05,
        effective_load: 0,
        last_selected_at: None,
        account_id: 21,
    };

    assert_eq!(
        compare_pool_routing_candidate_scores(&closer_secondary_reset, &farther_secondary_reset),
        std::cmp::Ordering::Less,
        "7-day reset proximity should sort before short-window reset and usage pressure",
    );
}

#[test]
fn reset_proximity_places_missing_reset_after_known_reset() {
    let known_reset = PoolRoutingCandidateScore {
        eligibility: PoolRoutingCandidateEligibility::Assignable,
        route_binding_failure_penalty: 0,
        model_route_penalty: 0,
        routing_priority_rank: 1,
        capacity_lane: PoolRoutingCandidateCapacityLane::Primary,
        dispatch_state: PoolRoutingCandidateDispatchState::ReadyOnOwnedNode,
        single_account_rotation_enabled: true,
        secondary_reset_proximity_secs: Some(60 * 5),
        primary_reset_proximity_secs: None,
        scarcity_score: 0.9,
        effective_load: 3,
        last_selected_at: Some("2026-03-23T12:00:00Z".to_string()),
        account_id: 22,
    };
    let missing_reset = PoolRoutingCandidateScore {
        eligibility: PoolRoutingCandidateEligibility::Assignable,
        route_binding_failure_penalty: 0,
        model_route_penalty: 0,
        routing_priority_rank: 1,
        capacity_lane: PoolRoutingCandidateCapacityLane::Primary,
        dispatch_state: PoolRoutingCandidateDispatchState::ReadyOnOwnedNode,
        single_account_rotation_enabled: true,
        secondary_reset_proximity_secs: None,
        primary_reset_proximity_secs: Some(1),
        scarcity_score: 0.0,
        effective_load: 0,
        last_selected_at: None,
        account_id: 23,
    };

    assert_eq!(
        compare_pool_routing_candidate_scores(&known_reset, &missing_reset),
        std::cmp::Ordering::Less,
        "known 7-day reset times should beat accounts without a 7-day reset time",
    );
}

#[test]
fn reset_proximity_does_not_change_default_sort_when_rotation_is_disabled() {
    let closer_secondary_reset = PoolRoutingCandidateScore {
        eligibility: PoolRoutingCandidateEligibility::Assignable,
        route_binding_failure_penalty: 0,
        model_route_penalty: 0,
        routing_priority_rank: 1,
        capacity_lane: PoolRoutingCandidateCapacityLane::Primary,
        dispatch_state: PoolRoutingCandidateDispatchState::ReadyOnOwnedNode,
        single_account_rotation_enabled: false,
        secondary_reset_proximity_secs: Some(60),
        primary_reset_proximity_secs: Some(60),
        scarcity_score: 0.95,
        effective_load: 2,
        last_selected_at: Some("2026-03-23T12:00:00Z".to_string()),
        account_id: 24,
    };
    let lower_pressure = PoolRoutingCandidateScore {
        eligibility: PoolRoutingCandidateEligibility::Assignable,
        route_binding_failure_penalty: 0,
        model_route_penalty: 0,
        routing_priority_rank: 1,
        capacity_lane: PoolRoutingCandidateCapacityLane::Primary,
        dispatch_state: PoolRoutingCandidateDispatchState::ReadyOnOwnedNode,
        single_account_rotation_enabled: false,
        secondary_reset_proximity_secs: Some(60 * 60 * 24),
        primary_reset_proximity_secs: Some(60 * 60 * 24),
        scarcity_score: 0.05,
        effective_load: 0,
        last_selected_at: None,
        account_id: 25,
    };

    assert_eq!(
        compare_pool_routing_candidate_scores(&closer_secondary_reset, &lower_pressure),
        std::cmp::Ordering::Greater,
        "disabled groups should still fall through to usage pressure tie-breakers",
    );
}

#[tokio::test]
async fn resolver_uses_reset_time_candidate_after_single_account_rotation_429() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let group_name = "single-rotation-reset-order";
    let sticky_key = "sticky-single-rotation-reset-order";
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let exhausted = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Rotation exhausted",
        "rotation-exhausted@example.com",
        "org_rotation_exhausted",
        "user_rotation_exhausted",
    )
    .await;
    let closer_reset = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Rotation closer reset",
        "rotation-closer-reset@example.com",
        "org_rotation_closer_reset",
        "user_rotation_closer_reset",
    )
    .await;
    let farther_reset = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Rotation farther reset",
        "rotation-farther-reset@example.com",
        "org_rotation_farther_reset",
        "user_rotation_farther_reset",
    )
    .await;
    set_test_account_group_name(&state.pool, exhausted, Some(group_name)).await;
    set_test_account_group_name(&state.pool, closer_reset, Some(group_name)).await;
    set_test_account_group_name(&state.pool, farther_reset, Some(group_name)).await;

    let now = Utc::now();
    let now_iso = format_utc_iso(now);
    sqlx::query(
        r#"
            INSERT INTO pool_upstream_account_group_notes (
                group_name, note, single_account_rotation_enabled, created_at, updated_at
            ) VALUES (?1, '', 1, ?2, ?2)
            ON CONFLICT(group_name) DO UPDATE SET
                single_account_rotation_enabled = excluded.single_account_rotation_enabled,
                updated_at = excluded.updated_at
            "#,
    )
    .bind(group_name)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .expect("enable single-account rotation for group");
    upsert_test_group_binding(
        &state.pool,
        group_name,
        vec![FORWARD_PROXY_DIRECT_KEY.to_string()],
    )
    .await;

    upsert_sticky_route(&state.pool, sticky_key, exhausted, &now_iso)
        .await
        .expect("seed sticky route");
    insert_limit_sample_with_reset_times(
        &state.pool,
        exhausted,
        &now_iso,
        Some(&format_utc_iso(now + ChronoDuration::minutes(5))),
        Some(&format_utc_iso(now + ChronoDuration::minutes(5))),
        1.0,
        1.0,
    )
    .await;
    insert_limit_sample_with_reset_times(
        &state.pool,
        closer_reset,
        &now_iso,
        Some(&format_utc_iso(now + ChronoDuration::hours(4))),
        Some(&format_utc_iso(now + ChronoDuration::minutes(30))),
        95.0,
        95.0,
    )
    .await;
    insert_limit_sample_with_reset_times(
        &state.pool,
        farther_reset,
        &now_iso,
        Some(&format_utc_iso(now + ChronoDuration::minutes(10))),
        Some(&format_utc_iso(now + ChronoDuration::days(2))),
        1.0,
        1.0,
    )
    .await;

    record_pool_route_http_failure(
        &state.pool,
        exhausted,
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        true,
        Some(sticky_key),
        StatusCode::TOO_MANY_REQUESTS,
        "pool upstream responded with 429: temporary rate limit",
        Some("invk_single_rotation_reset_order"),
    )
    .await
    .expect("record final 429");

    let sticky_after_failure: Option<i64> =
        sqlx::query_scalar("SELECT account_id FROM pool_sticky_routes WHERE sticky_key = ?1")
            .bind(sticky_key)
            .fetch_optional(&state.pool)
            .await
            .expect("load sticky route after 429");
    assert_eq!(sticky_after_failure, None);

    let resolution =
        resolve_pool_account_for_request(&state, Some(sticky_key), &[], &HashSet::new())
            .await
            .expect("resolve after final 429");
    let PoolAccountResolution::Resolved(account) = resolution.clone() else {
        panic!("expected resolver to select the next reset-time candidate, got {resolution:?}");
    };
    assert_eq!(account.account_id, closer_reset);
    assert!(account.single_account_rotation_enabled);
}

#[tokio::test]
async fn resolver_keeps_quota_exhausted_accounts_in_rate_limited_terminal_state_after_sync_block() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Quota Exhausted Resolver").await;
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    record_account_sync_recovery_blocked(
            &state.pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED,
            "manual recovery required because API key sync cannot verify whether the upstream usage limit has reset",
            Some("seed hard unavailable"),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED),
        )
        .await
        .expect("record blocked recovery");

    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve pool account");
    assert!(matches!(resolution, PoolAccountResolution::RateLimited));
}

#[tokio::test]
async fn resolver_skips_candidate_when_group_has_no_bound_proxy_keys() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let blocked = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Blocked Missing Binding",
        "blocked-missing-binding@example.com",
        "org_blocked_missing_binding",
        "user_blocked_missing_binding",
    )
    .await;
    let healthy = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Healthy Candidate",
        "healthy-candidate@example.com",
        "org_healthy_candidate",
        "user_healthy_candidate",
    )
    .await;
    set_test_account_group_name(&state.pool, blocked, Some("missing-bindings")).await;
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, blocked, &now_iso, Some(1.0), Some(1.0)).await;
    insert_limit_sample_with_usage(&state.pool, healthy, &now_iso, Some(80.0), Some(10.0)).await;

    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected resolver to skip missing-binding group and pick healthy account");
    };
    assert_eq!(account.account_id, healthy);
}

#[tokio::test]
async fn resolver_skips_candidate_when_group_has_only_unselectable_bound_proxies() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let blocked = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Blocked Unselectable Binding",
        "blocked-unselectable-binding@example.com",
        "org_blocked_unselectable_binding",
        "user_blocked_unselectable_binding",
    )
    .await;
    let healthy = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Healthy Fallback",
        "healthy-fallback@example.com",
        "org_healthy_fallback",
        "user_healthy_fallback",
    )
    .await;
    set_test_account_group_name(&state.pool, blocked, Some("staging")).await;
    upsert_test_group_binding(
        &state.pool,
        "staging",
        vec!["unselectable-bound-node".to_string()],
    )
    .await;
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, blocked, &now_iso, Some(1.0), Some(1.0)).await;
    insert_limit_sample_with_usage(&state.pool, healthy, &now_iso, Some(70.0), Some(10.0)).await;

    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected resolver to skip unselectable group and pick healthy account");
    };
    assert_eq!(account.account_id, healthy);
}

#[tokio::test]
async fn resolver_skips_ungrouped_candidate_when_healthy_grouped_account_exists() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let ungrouped = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Ungrouped Candidate",
        "ungrouped-candidate@example.com",
        "org_ungrouped_candidate",
        "user_ungrouped_candidate",
    )
    .await;
    let healthy = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Healthy Grouped Candidate",
        "healthy-grouped-candidate@example.com",
        "org_healthy_grouped_candidate",
        "user_healthy_grouped_candidate",
    )
    .await;
    set_test_account_group_name(&state.pool, ungrouped, None).await;
    let now_iso = format_utc_iso(Utc::now());
    insert_limit_sample_with_usage(&state.pool, ungrouped, &now_iso, Some(1.0), Some(1.0)).await;
    insert_limit_sample_with_usage(&state.pool, healthy, &now_iso, Some(80.0), Some(10.0)).await;

    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve pool account");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected resolver to skip ungrouped account and pick healthy grouped account");
    };
    assert_eq!(account.account_id, healthy);
}
