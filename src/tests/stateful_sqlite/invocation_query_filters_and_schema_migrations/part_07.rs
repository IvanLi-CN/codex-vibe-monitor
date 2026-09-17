#[tokio::test]
pub(crate) async fn prompt_cache_clear_and_reset_affinity_fences_stale_sticky_revival_and_emits_only_fresh_runtime_sticky_event()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let stale_account_id =
        insert_test_pool_api_key_account(&state, "Prompt Cache Stale Sticky", "upstream-stale")
            .await;
    let rebound_account_id =
        insert_test_pool_api_key_account(&state, "Prompt Cache Fresh Sticky", "upstream-fresh")
            .await;
    let prompt_cache_key = "prompt-cache-affinity-fence-key";
    let stale_invoke_id = "prompt-cache-stale-sticky-invoke";
    let fresh_invoke_id = "prompt-cache-fresh-sticky-invoke";
    let keepalive_invoke_id = "prompt-cache-keepalive-sticky-invoke";
    let seeded_at = format_utc_iso(Utc::now());

    let (stale_generation, fresh_generation) =
        reset_sticky_affinity(&state, prompt_cache_key, stale_account_id, &seeded_at).await;

    record_pool_route_success_with_affinity_generation(
        &state.pool,
        stale_account_id,
        Utc::now(),
        Some(prompt_cache_key),
        Some(prompt_cache_key),
        Some(stale_invoke_id),
        Some(stale_generation),
    )
    .await
    .expect("stale success writeback should be ignored");
    assert!(
        load_sticky_route(&state.pool, prompt_cache_key)
            .await
            .expect("load sticky route after stale success")
            .is_none()
    );

    record_pool_route_success_with_affinity_generation(
        &state.pool,
        rebound_account_id,
        Utc::now(),
        Some(prompt_cache_key),
        Some(prompt_cache_key),
        Some(fresh_invoke_id),
        Some(fresh_generation),
    )
    .await
    .expect("fresh success should create sticky route");
    let sticky_row = load_sticky_route(&state.pool, prompt_cache_key)
        .await
        .expect("load sticky route after fresh success")
        .expect("fresh success should rebuild sticky route");
    assert_eq!(sticky_row.account_id, rebound_account_id);
    let keepalive_generation = load_sticky_affinity_generation(&state.pool, prompt_cache_key)
        .await
        .expect("load affinity generation after fresh success");
    assert_eq!(keepalive_generation, fresh_generation + 1);

    record_pool_route_success_with_affinity_generation(
        &state.pool,
        rebound_account_id,
        Utc::now(),
        Some(prompt_cache_key),
        Some(prompt_cache_key),
        Some(keepalive_invoke_id),
        Some(keepalive_generation),
    )
    .await
    .expect("same-account keepalive should refresh without a new event");

    assert_fresh_runtime_sticky_event(
        state,
        prompt_cache_key,
        stale_invoke_id,
        fresh_invoke_id,
        keepalive_invoke_id,
        rebound_account_id,
    )
    .await;
}

async fn reset_sticky_affinity(
    state: &Arc<AppState>,
    prompt_cache_key: &str,
    stale_account_id: i64,
    seeded_at: &str,
) -> (i64, i64) {
    upsert_sticky_route(&state.pool, prompt_cache_key, stale_account_id, seeded_at)
        .await
        .expect("seed sticky route before clear");
    let stale_generation = load_sticky_affinity_generation(&state.pool, prompt_cache_key)
        .await
        .expect("load affinity generation before clear");
    assert_eq!(stale_generation, 0);
    let payload = serde_json::from_value(json!({
        "promptCacheKeys": [prompt_cache_key], "action": "clearAndResetAffinity",
    }))
    .expect("deserialize clear payload");
    let Json(response) =
        post_bulk_prompt_cache_conversation_bindings(State(state.clone()), Json(payload))
            .await
            .expect("clear and reset affinity should succeed");
    assert_eq!(response.action, "clearAndResetAffinity");
    assert_eq!(response.total_succeeded, 1);
    let fresh_generation = load_sticky_affinity_generation(&state.pool, prompt_cache_key)
        .await
        .expect("load affinity generation after clear");
    assert_eq!(fresh_generation, stale_generation + 1);
    assert!(
        load_sticky_route(&state.pool, prompt_cache_key)
            .await
            .expect("load sticky route after clear")
            .is_none()
    );
    (stale_generation, fresh_generation)
}

async fn assert_fresh_runtime_sticky_event(
    state: Arc<AppState>,
    prompt_cache_key: &str,
    stale_invoke_id: &str,
    fresh_invoke_id: &str,
    keepalive_invoke_id: &str,
    rebound_account_id: i64,
) {
    let Json(event_response) = list_prompt_cache_conversation_operation_events(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        axum::extract::Query(ListPromptCacheConversationOperationEventsQuery {
            page: Some(1),
            page_size: Some(20),
            info_type: None,
            routing_scope: None,
            routing_model: None,
        }),
    )
    .await
    .expect("list prompt cache operation events after stale and fresh successes");

    assert!(
        !event_response.items.iter().any(|event| {
            event.action == "stickyTargetChanged"
                && event.origin == "systemAuto"
                && event.invoke_id.as_deref() == Some(stale_invoke_id)
        }),
        "stale in-flight success must not resurrect sticky ownership"
    );

    let fresh_events = event_response
        .items
        .iter()
        .filter(|event| event.action == "stickyTargetChanged" && event.origin == "systemAuto")
        .collect::<Vec<_>>();
    assert_eq!(
        fresh_events.len(),
        1,
        "fresh assignment should emit exactly one runtime sticky change event"
    );
    let fresh_event = fresh_events[0];
    assert_eq!(fresh_event.invoke_id.as_deref(), Some(fresh_invoke_id));
    assert_eq!(fresh_event.info_types, vec!["routing".to_string()]);
    assert_eq!(
        fresh_event
            .sticky_before
            .as_ref()
            .map(|sticky| sticky.upstream_account_id),
        None
    );
    assert_eq!(
        fresh_event
            .sticky_after
            .as_ref()
            .map(|sticky| sticky.upstream_account_id),
        Some(rebound_account_id)
    );
    assert!(
        !event_response.items.iter().any(|event| {
            event.action == "stickyTargetChanged"
                && event.origin == "systemAuto"
                && event.invoke_id.as_deref() == Some(keepalive_invoke_id)
        }),
        "same-account keepalive should not emit extra sticky change noise"
    );
}

#[tokio::test]
pub(crate) async fn prompt_cache_sticky_routes_are_isolated_by_normalized_model_key() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let fallback_account_id =
        insert_test_pool_api_key_account(&state, "Prompt Cache Fallback", "model-fallback").await;
    let model_account_id =
        insert_test_pool_api_key_account(&state, "Prompt Cache Model", "model-specific").await;
    let prompt_cache_key = "prompt-cache-model-isolation-key";
    let now_iso = format_utc_iso(Utc::now());

    upsert_sticky_route(&state.pool, prompt_cache_key, fallback_account_id, &now_iso)
        .await
        .expect("seed all-model fallback");
    assert!(
        upsert_sticky_route_for_model_if_current(
            &state.pool,
            prompt_cache_key,
            Some(" GPT-5.4-2026-05-01 "),
            model_account_id,
            None,
            &now_iso,
        )
        .await
        .expect("write normalized model route"),
        "the first model-specific success should materialize its exact bucket"
    );

    let (dated_alias_route, _) =
        load_sticky_route_with_model_generation(&state.pool, prompt_cache_key, Some("gpt-5.4"))
            .await
            .expect("load normalized alias route");
    assert_eq!(
        dated_alias_route.map(|route| route.account_id),
        Some(model_account_id),
        "a dated alias and its base model must share one Sticky bucket"
    );

    let (other_model_route, _) = load_sticky_route_with_model_generation(
        &state.pool,
        prompt_cache_key,
        Some("gpt-5.1-codex-max"),
    )
    .await
    .expect("load fallback route for unrelated model");
    assert_eq!(
        other_model_route.map(|route| route.account_id),
        Some(fallback_account_id),
        "an unrelated model must not inherit another model's exact Sticky route"
    );
}

#[tokio::test]
pub(crate) async fn prompt_cache_manual_binding_is_idempotent_for_unchanged_model_routes() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id =
        insert_test_pool_api_key_account(&state, "Prompt Cache Manual", "manual-model-route").await;
    let prompt_cache_key = "prompt-cache-manual-idempotent-model-route";
    let initial_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "upstreamAccount",
            "upstreamAccountId": account_id,
        }))
        .expect("deserialize manual binding payload");

    let _ = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(initial_payload),
    )
    .await
    .expect("save initial manual binding");
    let now_iso = format_utc_iso(Utc::now());
    assert!(
        upsert_sticky_route_for_model_if_current(
            &state.pool,
            prompt_cache_key,
            Some("gpt-5.4"),
            account_id,
            None,
            &now_iso,
        )
        .await
        .expect("materialize exact model route")
    );
    let epoch_before = load_sticky_affinity_generation(&state.pool, prompt_cache_key)
        .await
        .expect("load manual binding epoch");
    let (_, model_generation_before) =
        load_sticky_route_with_model_generation(&state.pool, prompt_cache_key, Some("gpt-5.4"))
            .await
            .expect("load exact model generation");

    let repeat_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "upstreamAccount",
            "upstreamAccountId": account_id,
        }))
        .expect("deserialize repeated manual binding payload");

    let _ = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(repeat_payload),
    )
    .await
    .expect("repeat manual binding without changing routes");

    assert_eq!(
        load_sticky_affinity_generation(&state.pool, prompt_cache_key)
            .await
            .expect("load epoch after idempotent binding"),
        epoch_before,
        "a no-op binding must not fence in-flight requests"
    );
    let (_, model_generation_after) =
        load_sticky_route_with_model_generation(&state.pool, prompt_cache_key, Some("gpt-5.4"))
            .await
            .expect("load model generation after idempotent binding");
    assert_eq!(model_generation_after, model_generation_before);
}

#[tokio::test]
pub(crate) async fn prompt_cache_manual_binding_rolls_back_when_sticky_update_fails() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id =
        insert_test_pool_api_key_account(&state, "Prompt Cache Atomic", "atomic-manual-route")
            .await;
    let prompt_cache_key = "prompt-cache-manual-binding-atomic";
    sqlx::query(
        r#"
        CREATE TRIGGER fail_prompt_cache_manual_sticky_insert
        BEFORE INSERT ON pool_sticky_routes
        WHEN NEW.sticky_key = 'prompt-cache-manual-binding-atomic'
        BEGIN
            SELECT RAISE(ABORT, 'forced sticky write failure');
        END
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("install sticky write failure trigger");
    let payload: UpdatePromptCacheConversationBindingRequest = serde_json::from_value(json!({
        "bindingKind": "upstreamAccount",
        "upstreamAccountId": account_id,
    }))
    .expect("deserialize atomic manual binding payload");

    let error = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(payload),
    )
    .await
    .expect_err("sticky failure must fail the account binding transaction");
    assert!(matches!(error, ApiError::Internal(_)));
    assert!(
        load_prompt_cache_conversation_binding_row(&state.pool, prompt_cache_key)
            .await
            .expect("load binding after failed transaction")
            .is_none(),
        "the binding row must roll back with the Sticky write"
    );
    let sticky_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pool_sticky_routes WHERE sticky_key = ?1")
            .bind(prompt_cache_key)
            .fetch_one(&state.pool)
            .await
            .expect("count sticky rows after failed transaction");
    assert_eq!(sticky_count, 0);
}

#[tokio::test]
pub(crate) async fn prompt_cache_routing_events_filter_by_all_or_normalized_model_scope() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let prompt_cache_key = "prompt-cache-routing-event-model-filter";
    for (action, scope) in [
        ("legacyAll", r#"{"kind":"all"}"#),
        (
            "gpt54",
            r#"{"kind":"model","modelKey":"gpt-5.4","requestModel":"gpt-5.4-2026-05-01"}"#,
        ),
        (
            "gpt51",
            r#"{"kind":"model","modelKey":"gpt-5.1-codex-max"}"#,
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO prompt_cache_conversation_operation_events (
                prompt_cache_key, action, origin, info_types_json, occurred_at, headline,
                routing_scope_json
            ) VALUES (?1, ?2, 'systemAuto', '["routing"]', datetime('now'), ?2, ?3)
            "#,
        )
        .bind(prompt_cache_key)
        .bind(action)
        .bind(scope)
        .execute(&state.pool)
        .await
        .expect("seed scoped routing event");
    }

    let Json(model_events) = list_prompt_cache_conversation_operation_events(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        axum::extract::Query(ListPromptCacheConversationOperationEventsQuery {
            page: Some(1),
            page_size: Some(20),
            info_type: Some("routing".to_string()),
            routing_scope: Some("model".to_string()),
            routing_model: Some(" GPT-5.4-2026-05-01 ".to_string()),
        }),
    )
    .await
    .expect("filter routing events by normalized model");
    assert_eq!(model_events.total, 1);
    assert_eq!(model_events.items[0].action, "gpt54");
    assert_eq!(
        model_events.routing_model_facets,
        vec!["gpt-5.1-codex-max".to_string(), "gpt-5.4".to_string()],
        "facets are derived from all retained events rather than the active filter"
    );

    let Json(all_scope_events) = list_prompt_cache_conversation_operation_events(
        State(state),
        AxumPath(prompt_cache_key.to_string()),
        axum::extract::Query(ListPromptCacheConversationOperationEventsQuery {
            page: Some(1),
            page_size: Some(20),
            info_type: Some("routing".to_string()),
            routing_scope: Some("all".to_string()),
            routing_model: None,
        }),
    )
    .await
    .expect("filter routing events by all-model scope");
    assert_eq!(all_scope_events.total, 1);
    assert_eq!(all_scope_events.items[0].action, "legacyAll");
}

#[tokio::test]
pub(crate) async fn fresh_assignment_persists_selection_audit_for_attempt_and_sticky_event() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let prompt_cache_key = "prompt-cache-selection-audit-key";
    let invoke_id = "selection-audit-invoke";
    let (selected_account_id, attempt_id) =
        seed_selection_audit_attempt(&state, prompt_cache_key, invoke_id).await;
    let generation = load_sticky_affinity_generation(&state.pool, prompt_cache_key)
        .await
        .expect("load empty sticky generation");

    record_pool_route_success_with_affinity_generation_for_attempt(
        &state.pool,
        selected_account_id,
        Utc::now(),
        Some(prompt_cache_key),
        Some(prompt_cache_key),
        None,
        Some(attempt_id),
        Some(generation),
    )
    .await
    .expect("persist fresh sticky success");

    assert_selection_audit_surfaces(state, prompt_cache_key, invoke_id).await;
}

async fn seed_selection_audit_attempt(
    state: &Arc<AppState>,
    prompt_cache_key: &str,
    invoke_id: &str,
) -> (i64, i64) {
    let selected_account_id =
        insert_test_pool_api_key_account(state, "Selected dzw", "upstream-selected").await;
    let excluded_account_id =
        insert_test_pool_api_key_account(state, "Excluded CIII", "upstream-excluded").await;
    let occurred_at = format_utc_iso(Utc::now());
    let audit = PoolRoutingSelectionAudit {
        selected_account_id,
        selected_account_name: "Selected dzw".to_string(),
        eligible_candidate_count: 1,
        winner_reason_code: "onlyEligibleCandidate".to_string(),
        compared_account_id: None,
        compared_account_name: None,
        selected_score: None,
        compared_score: None,
        handoff_admission: None,
        excluded_candidates: vec![PoolRoutingSelectionAuditExcludedCandidate {
            account_id: excluded_account_id,
            account_name: "Excluded CIII".to_string(),
            reason_code: "modelNotAllowed".to_string(),
        }],
    };
    let attempt_id = sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            attempt_public_id, invoke_id, occurred_at, endpoint, route_mode, sticky_key,
            routing_source, request_model, routing_selection_audit_json, upstream_account_id,
            upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index,
            requester_ip, started_at, finished_at, status, phase, created_at
        ) VALUES (
            ?1, ?2, ?3, '/v1/responses', ?4, ?5, 'freshAssignment', ?6, ?7, ?8,
            'route-selection-audit', 1, 1, 0, '203.0.113.15', ?3, ?3,
            'success', 'completed', ?3
        )
        "#,
    )
    .bind("SELECTAUDIT1")
    .bind(invoke_id)
    .bind(&occurred_at)
    .bind(INVOCATION_ROUTE_MODE_POOL)
    .bind(prompt_cache_key)
    .bind("gpt-5.4")
    .bind(serde_json::to_string(&audit).expect("serialize selection audit"))
    .bind(selected_account_id)
    .execute(&state.pool)
    .await
    .expect("insert attempt with selection audit")
    .last_insert_rowid();
    (selected_account_id, attempt_id)
}

async fn assert_selection_audit_surfaces(
    state: Arc<AppState>,
    prompt_cache_key: &str,
    invoke_id: &str,
) {
    let Json(attempts) =
        fetch_invocation_pool_attempts(State(state.clone()), AxumPath(invoke_id.to_string()))
            .await
            .expect("fetch attempt with selection audit");
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0]
            .routing_selection_audit
            .as_ref()
            .map(|value| value.selected_account_name.as_str()),
        Some("Selected dzw")
    );
    assert_eq!(
        attempts[0]
            .routing_selection_audit
            .as_ref()
            .map(|value| value.excluded_candidates[0].reason_code.as_str()),
        Some("modelNotAllowed")
    );

    let Json(events) = list_prompt_cache_conversation_operation_events(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        axum::extract::Query(ListPromptCacheConversationOperationEventsQuery {
            page: Some(1),
            page_size: Some(20),
            info_type: None,
            routing_scope: None,
            routing_model: None,
        }),
    )
    .await
    .expect("list sticky event with selection audit");
    let event = events
        .items
        .iter()
        .find(|event| event.action == "stickyTargetChanged")
        .expect("fresh assignment should emit sticky target event");
    assert_eq!(
        event
            .routing_context
            .as_ref()
            .and_then(|context| context.routing_selection_audit.as_ref())
            .map(|value| value.winner_reason_code.as_str()),
        Some("onlyEligibleCandidate")
    );
    assert_eq!(event.invoke_id.as_deref(), Some(invoke_id));
    assert_eq!(
        event
            .routing_scope
            .as_ref()
            .and_then(|scope| scope.request_model.as_deref()),
        None,
        "requestModel duplicates the normalized model key and should be omitted"
    );
}

#[tokio::test]
pub(crate) async fn model_scoped_sticky_clear_cause_does_not_cross_models() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let cleared_account_id =
        insert_test_pool_api_key_account(&state, "Cleared model account", "upstream-cleared").await;
    let replacement_account_id = insert_test_pool_api_key_account(
        &state,
        "Independent model account",
        "upstream-independent",
    )
    .await;
    let prompt_cache_key = "prompt-cache-model-clear-cause-scope";
    let now_iso = format_utc_iso(Utc::now());

    seed_model_scoped_clear_cause(&state, prompt_cache_key, cleared_account_id, &now_iso).await;

    let replacement_attempt_id = sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            attempt_public_id, invoke_id, occurred_at, endpoint, route_mode, sticky_key,
            routing_source, request_model, upstream_account_id, upstream_route_key,
            attempt_index, distinct_account_index, same_account_retry_index, requester_ip,
            started_at, finished_at, status, phase, created_at
        ) VALUES (
            'SUCCESS51', 'success-gpt-51', ?1, '/v1/responses', ?2, ?3,
            'freshAssignment', 'gpt-5.1-codex-max', ?4, 'route-success-51',
            1, 1, 0, '203.0.113.51', ?1, ?1, 'success', 'completed', ?1
        )
        "#,
    )
    .bind(&now_iso)
    .bind(INVOCATION_ROUTE_MODE_POOL)
    .bind(prompt_cache_key)
    .bind(replacement_account_id)
    .execute(&state.pool)
    .await
    .expect("insert gpt-5.1 success attempt")
    .last_insert_rowid();

    assert!(matches!(
        upsert_runtime_prompt_cache_conversation_sticky_route(
            &state.pool,
            prompt_cache_key,
            Some(prompt_cache_key),
            replacement_account_id,
            &now_iso,
            Some("success-gpt-51"),
            Some(replacement_attempt_id),
            None,
        )
        .await
        .expect("persist independent model sticky route"),
        RuntimeStickyMutation::Changed {
            previous_upstream_account_id: None
        }
    ));

    assert_independent_model_assignment_event(state, prompt_cache_key).await;
}

async fn seed_model_scoped_clear_cause(
    state: &Arc<AppState>,
    prompt_cache_key: &str,
    cleared_account_id: i64,
    now_iso: &str,
) {
    upsert_sticky_route_for_model_if_current(
        &state.pool,
        prompt_cache_key,
        Some("gpt-5.4"),
        cleared_account_id,
        None,
        now_iso,
    )
    .await
    .expect("seed model-scoped sticky route");
    let (_, gpt54_token) =
        load_sticky_route_with_model_generation(&state.pool, prompt_cache_key, Some("gpt-5.4"))
            .await
            .expect("load gpt-5.4 sticky token");

    let clear_attempt_id = sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            attempt_public_id, invoke_id, occurred_at, endpoint, route_mode, sticky_key,
            routing_source, request_model, upstream_account_id, upstream_route_key,
            attempt_index, distinct_account_index, same_account_retry_index, requester_ip,
            started_at, finished_at, status, phase, created_at
        ) VALUES (
            'CLEAR54', 'clear-gpt-54', ?1, '/v1/responses', ?2, ?3,
            'stickyRoute', 'gpt-5.4', ?4, 'route-clear-54',
            1, 1, 0, '203.0.113.54', ?1, ?1, 'failed', 'completed', ?1
        )
        "#,
    )
    .bind(now_iso)
    .bind(INVOCATION_ROUTE_MODE_POOL)
    .bind(prompt_cache_key)
    .bind(cleared_account_id)
    .execute(&state.pool)
    .await
    .expect("insert gpt-5.4 clear attempt")
    .last_insert_rowid();

    assert!(
        delete_sticky_route_if_matches_with_cause(
            &state.pool,
            prompt_cache_key,
            cleared_account_id,
            Some(gpt54_token),
            Some(clear_attempt_id),
            Some(429),
            Some("upstreamHttp429"),
            Some(prompt_cache_key),
            now_iso,
        )
        .await
        .expect("clear gpt-5.4 sticky route"),
        "the matching model bucket should be cleared"
    );

    let gpt54_clear_cause = sqlx::query_as::<_, (Option<String>, Option<i64>)>(
        r#"
        SELECT last_clear_cause_attempt_public_id, last_clear_cause_http_status
        FROM pool_sticky_model_route_generations
        WHERE sticky_key = ?1 AND model_key = 'gpt-5.4'
        "#,
    )
    .bind(prompt_cache_key)
    .fetch_one(&state.pool)
    .await
    .expect("load gpt-5.4 clear cause");
    assert_eq!(gpt54_clear_cause.0.as_deref(), Some("CLEAR54"));
    assert_eq!(gpt54_clear_cause.1, Some(429));
}

async fn assert_independent_model_assignment_event(state: Arc<AppState>, prompt_cache_key: &str) {
    let Json(events) = list_prompt_cache_conversation_operation_events(
        State(state),
        AxumPath(prompt_cache_key.to_string()),
        axum::extract::Query(ListPromptCacheConversationOperationEventsQuery {
            page: Some(1),
            page_size: Some(20),
            info_type: None,
            routing_scope: None,
            routing_model: None,
        }),
    )
    .await
    .expect("list model-scoped sticky events");
    let replacement_event = events
        .items
        .iter()
        .find(|event| event.invoke_id.as_deref() == Some("success-gpt-51"))
        .expect("gpt-5.1 success event should be present");
    assert_eq!(
        replacement_event
            .routing_context
            .as_ref()
            .map(|context| context.reason_code.as_str()),
        Some("firstSuccessfulAssignment")
    );
    assert_eq!(
        replacement_event
            .routing_context
            .as_ref()
            .and_then(|context| context.causing_attempt_id.as_deref()),
        None,
        "a gpt-5.4 clear must not become the cause of a different model's assignment"
    );
}

#[tokio::test]
pub(crate) async fn runtime_sticky_first_concurrent_success_locks_target_and_audits_late_completion()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let first_account_id =
        insert_test_pool_api_key_account(&state, "First Sticky Winner", "upstream-first").await;
    let late_account_id =
        insert_test_pool_api_key_account(&state, "Late Sticky Loser", "upstream-late").await;
    let prompt_cache_key = "prompt-cache-first-success-lock-key";
    let mut broadcast_receiver = state.broadcaster.subscribe();
    let generation = load_sticky_affinity_generation(&state.pool, prompt_cache_key)
        .await
        .expect("load empty sticky generation");

    record_pool_route_success_with_affinity_generation_and_broadcast(
        state.as_ref(),
        first_account_id,
        Utc::now(),
        Some(prompt_cache_key),
        Some(prompt_cache_key),
        Some("first-success"),
        None,
        Some(generation),
    )
    .await
    .expect("first success should establish sticky target");
    assert!(matches!(
        broadcast_receiver
            .recv()
            .await
            .expect("first sticky mutation should broadcast"),
        BroadcastPayload::PromptCacheConversationChanged { prompt_cache_key: key }
            if key == prompt_cache_key
    ));
    record_pool_route_success_with_affinity_generation_and_broadcast(
        state.as_ref(),
        late_account_id,
        Utc::now(),
        Some(prompt_cache_key),
        Some(prompt_cache_key),
        Some("late-success"),
        None,
        Some(generation),
    )
    .await
    .expect("late success remains a successful route outcome");
    assert!(matches!(
        broadcast_receiver
            .recv()
            .await
            .expect("suppressed sticky mutation should broadcast"),
        BroadcastPayload::PromptCacheConversationChanged { prompt_cache_key: key }
            if key == prompt_cache_key
    ));

    let sticky = load_sticky_route(&state.pool, prompt_cache_key)
        .await
        .expect("load sticky route")
        .expect("first success should leave sticky route");
    assert_eq!(sticky.account_id, first_account_id);
    let Json(events) = list_prompt_cache_conversation_operation_events(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        axum::extract::Query(ListPromptCacheConversationOperationEventsQuery {
            page: Some(1),
            page_size: Some(20),
            info_type: None,
            routing_scope: None,
            routing_model: None,
        }),
    )
    .await
    .expect("list runtime sticky events");
    assert_eq!(
        events
            .items
            .iter()
            .filter(|event| event.action == "stickyTargetChanged")
            .count(),
        1
    );
    let suppressed = events
        .items
        .iter()
        .find(|event| event.action == "stickyMutationSuppressed")
        .expect("late completion should be audited");
    assert_eq!(suppressed.invoke_id.as_deref(), Some("late-success"));
    assert_eq!(
        suppressed
            .routing_context
            .as_ref()
            .map(|context| context.reason_code.as_str()),
        Some("staleConcurrentCompletion")
    );
}

#[tokio::test]
pub(crate) async fn runtime_sticky_route_change_broadcasts_previous_and_current_history_scopes() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let previous_account_id =
        insert_test_pool_api_key_account(&state, "Previous Sticky Target", "upstream-previous")
            .await;
    let current_account_id =
        insert_test_pool_api_key_account(&state, "Current Sticky Target", "upstream-current").await;
    let prompt_cache_key = "prompt-cache-sticky-history-invalidation";
    upsert_sticky_route(
        &state.pool,
        prompt_cache_key,
        previous_account_id,
        &format_utc_iso(Utc::now()),
    )
    .await
    .expect("seed previous sticky target");
    let generation = load_sticky_affinity_generation(&state.pool, prompt_cache_key)
        .await
        .expect("load seeded sticky generation");
    let mut broadcast_receiver = state.broadcaster.subscribe();

    record_pool_route_success_with_affinity_generation_and_broadcast(
        state.as_ref(),
        current_account_id,
        Utc::now(),
        Some(prompt_cache_key),
        Some(prompt_cache_key),
        Some("sticky-route-change"),
        None,
        Some(generation),
    )
    .await
    .expect("route success should move sticky target");

    let first = broadcast_receiver
        .recv()
        .await
        .expect("sticky mutation should publish configuration change");
    let second = broadcast_receiver
        .recv()
        .await
        .expect("sticky mutation should publish history invalidation");
    assert!(
        matches!(
            &first,
            BroadcastPayload::PromptCacheConversationChanged { prompt_cache_key: key }
                if key == prompt_cache_key
        ) || matches!(
            &second,
            BroadcastPayload::PromptCacheConversationChanged { prompt_cache_key: key }
                if key == prompt_cache_key
        )
    );
    assert!(
        matches!(
            &first,
            BroadcastPayload::PromptCacheConversationStickyRouteChanged {
                sticky_key,
                previous_upstream_account_id,
                upstream_account_id,
            } if sticky_key == prompt_cache_key
                && *previous_upstream_account_id == previous_account_id
                && *upstream_account_id == current_account_id
        ) || matches!(
            &second,
            BroadcastPayload::PromptCacheConversationStickyRouteChanged {
                sticky_key,
                previous_upstream_account_id,
                upstream_account_id,
            } if sticky_key == prompt_cache_key
                && *previous_upstream_account_id == previous_account_id
                && *upstream_account_id == current_account_id
        )
    );
}

#[tokio::test]
pub(crate) async fn runtime_sticky_upsert_rechecks_generation_after_waiting_for_write_lock() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let stale_account_id =
        insert_test_pool_api_key_account(&state, "Prompt Cache Locked Stale", "upstream-locked")
            .await;
    let prompt_cache_key = "prompt-cache-affinity-lock-race-key";
    let seeded_at = format_utc_iso(Utc::now());

    upsert_sticky_route(&state.pool, prompt_cache_key, stale_account_id, &seeded_at)
        .await
        .expect("seed sticky route before lock race");
    let stale_generation = load_sticky_affinity_generation(&state.pool, prompt_cache_key)
        .await
        .expect("load affinity generation before lock race");
    assert_eq!(stale_generation, 0);

    let mut conn = state
        .pool
        .acquire()
        .await
        .expect("acquire connection for affinity reset lock");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(conn.as_mut())
        .await
        .expect("begin immediate affinity reset lock");
    let reset_now = format_utc_iso(Utc::now());
    bump_sticky_affinity_generation_executor(conn.as_mut(), prompt_cache_key, &reset_now)
        .await
        .expect("bump affinity generation while lock is held");
    delete_sticky_route_executor(conn.as_mut(), prompt_cache_key)
        .await
        .expect("delete sticky route while lock is held");

    let pool = state.pool.clone();
    let sticky_key = prompt_cache_key.to_string();
    let write_task = tokio::spawn(async move {
        upsert_runtime_prompt_cache_conversation_sticky_route(
            &pool,
            &sticky_key,
            Some(&sticky_key),
            stale_account_id,
            &format_utc_iso(Utc::now()),
            Some("locked-stale-invoke"),
            None,
            Some(stale_generation),
        )
        .await
    });

    sleep(Duration::from_millis(50)).await;
    sqlx::query("COMMIT")
        .execute(conn.as_mut())
        .await
        .expect("commit affinity reset lock");
    drop(conn);

    let sticky_updated = write_task
        .await
        .expect("join locked stale sticky write task")
        .expect("locked stale sticky write task should return result");
    assert_eq!(
        sticky_updated,
        RuntimeStickyMutation::Suppressed,
        "stale sticky writeback must be rejected after the reset transaction commits"
    );
    assert!(
        load_sticky_route(&state.pool, prompt_cache_key)
            .await
            .expect("load sticky route after locked stale writeback")
            .is_none(),
        "stale sticky writeback must not recreate sticky route after waiting on the reset lock"
    );
}

use super::*;
