use super::*;

async fn oauth(state: &AppState, name: &str) -> i64 {
    insert_syncable_oauth_account(
        &state.pool,
        state.upstream_accounts.crypto_key.as_ref().unwrap(),
        name,
        &format!("{name}@example.com"),
        name,
        name,
    )
    .await
}

#[tokio::test]
async fn oauth_model_routing_isolates_failures_exports_union_and_restores_without_deleting_history()
{
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account = oauth(&state, "oauth-model-a").await;
    let other = oauth(&state, "oauth-model-b").await;
    sqlx::query("UPDATE pool_upstream_accounts SET policy_available_models_mode='allowlist', policy_available_models_json='[\"policy-only\"]' WHERE id=?1")
        .bind(account).execute(&state.pool).await.unwrap();
    persist_model_catalog_success(
        &state.pool,
        account,
        &["catalog-only".into()],
        &format_utc_iso(Utc::now()),
    )
    .await
    .unwrap();
    record_model_route_success_for_model_admitted(&state.pool, account, "healthy", None)
        .await
        .unwrap();
    observe_model_route_seen(&state.pool, other, Some("limited"))
        .await
        .unwrap();
    let attempt =
        insert_model_failure_attempt(&state, account, "oauth-limited", Some("limited")).await;
    for _ in 0..5 {
        record_pool_route_http_failure_for_endpoint_with_image_intent_for_attempt(
            &state.pool,
            account,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            false,
            None,
            StatusCode::TOO_MANY_REQUESTS,
            "model rate limit exceeded",
            Some("oauth-limited"),
            "/v1/responses",
            ImageIntent::Unknown,
            Some(attempt),
            None,
        )
        .await
        .unwrap();
    }
    assert_eq!(
        model_route_penalty(&state.pool, account, Some("limited"))
            .await
            .unwrap(),
        ModelRoutePenalty::Excluded
    );
    assert_eq!(
        model_route_penalty(&state.pool, account, Some("healthy"))
            .await
            .unwrap(),
        ModelRoutePenalty::Normal
    );
    assert_eq!(
        model_route_penalty(&state.pool, other, Some("limited"))
            .await
            .unwrap(),
        ModelRoutePenalty::Normal
    );
    let row = load_upstream_account_row(&state.pool, account)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(row.last_route_failure_kind.is_none());
    let Json(states) = get_upstream_account_model_routing(State(state.clone()), AxumPath(account))
        .await
        .unwrap();
    assert!(
        states
            .iter()
            .any(|route| route.model == "catalog-only" && route.state == "unknown")
    );
    assert!(states.iter().any(|route| route.model == "policy-only"));
    assert!(
        states
            .iter()
            .any(|route| route.model == "limited" && route.state == MODEL_ROUTE_STATE_COOLING_DOWN)
    );
    let Json(history) = list_upstream_account_model_routing_events(
        State(state.clone()),
        AxumPath(account),
        Query(ModelRoutingHistoryQuery {
            model: "limited".into(),
            cursor: None,
            page_size: Some(50),
        }),
    )
    .await
    .unwrap();
    assert!(!history.items.is_empty());
    assert!(
        history
            .items
            .iter()
            .all(|event| event.account_id == account && event.model == "limited")
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pool_upstream_account_events WHERE account_id=?1 AND model='limited'",
    )
    .bind(account)
    .fetch_one(&state.pool)
    .await
    .unwrap();
    let Json(restored) = reset_upstream_account_model_routing(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(account),
        Json(ResetModelRoutingRequest {
            model: "limited".into(),
        }),
    )
    .await
    .unwrap();
    assert_eq!(restored.state, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(restored.failure_count, 0);
    assert!(restored.cooldown_until.is_none());
    let after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pool_upstream_account_events WHERE account_id=?1 AND model='limited'",
    )
    .bind(account)
    .fetch_one(&state.pool)
    .await
    .unwrap();
    assert!(count > 0 && after >= count);
    assert_eq!(
        model_route_penalty(&state.pool, other, Some("limited"))
            .await
            .unwrap(),
        ModelRoutePenalty::Normal
    );
}

#[tokio::test]
async fn oauth_runtime_model_failure_is_scoped_and_account_failure_remains_account_wide() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account = oauth(&state, "oauth-runtime").await;
    for _ in 0..5 {
        record_runtime_pool_route_http_failure(
            &state,
            account,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            false,
            None,
            StatusCode::TOO_MANY_REQUESTS,
            "model quota exceeded",
            None,
            "/v1/responses",
            ImageIntent::Unknown,
            None,
            None,
            None,
            Some("limited"),
        )
        .await
        .unwrap();
    }
    let rows = load_runtime_model_penalties(&state, &[account], Some("limited"))
        .await
        .unwrap();
    assert_eq!(rows.get(&account), Some(&ModelRoutePenalty::Excluded));
    let sibling = load_runtime_model_penalties(&state, &[account], Some("healthy"))
        .await
        .unwrap();
    assert_ne!(sibling.get(&account), Some(&ModelRoutePenalty::Excluded));
    let row = load_runtime_route_account(&state, account)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.consecutive_route_failures, 0);
    record_runtime_pool_route_http_failure(
        &state,
        account,
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        false,
        None,
        StatusCode::TOO_MANY_REQUESTS,
        "rate limit reached for this account",
        None,
        "/v1/responses",
        ImageIntent::Unknown,
        None,
        None,
        None,
        Some("healthy"),
    )
    .await
    .unwrap();
    let throttled = load_runtime_route_account(&state, account)
        .await
        .unwrap()
        .unwrap();
    assert!(throttled.consecutive_route_failures > 0);
    assert_ne!(
        load_runtime_model_penalties(&state, &[account], Some("healthy"))
            .await
            .unwrap()
            .get(&account),
        Some(&ModelRoutePenalty::Excluded)
    );
    record_runtime_pool_route_http_failure(
        &state,
        account,
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        false,
        None,
        StatusCode::UNAUTHORIZED,
        "invalid authentication credentials",
        None,
        "/v1/responses",
        ImageIntent::Unknown,
        None,
        None,
        None,
        Some("healthy"),
    )
    .await
    .unwrap();
    let row = load_runtime_route_account(&state, account)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(row.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
}
