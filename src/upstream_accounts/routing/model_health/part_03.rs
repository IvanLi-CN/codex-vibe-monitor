pub(crate) async fn reset_model_route(
    pool: &Pool<Sqlite>,
    account_id: i64,
    model: &str,
) -> Result<Option<ModelRoutingState>> {
    if !account_is_api_key(load_account_kind(pool, account_id).await?.as_deref()) {
        return Ok(None);
    }
    let model = model.trim();
    let Some(row) = sqlx::query_as::<_, ModelRouteRow>(
        "SELECT account_id, model, state, priority, consecutive_failures, streak_started_at, changed_at, last_seen_at, last_success_at, last_failure_at, last_failure_kind, last_failure_message, cooldown_until, reset_fence_at, cache_concurrency_limit, cache_recovery_limit, cache_low_hit_streak, cache_cooldown_level, cache_last_hit_rate_percent, cache_usage_missing_since, cache_usage_missing_reason FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .fetch_optional(pool)
    .await? else { return Ok(None) };
    let (before_state, before_priority, _) = effective_row_state(&row, Utc::now());
    let now = now_string();
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET state = ?3, priority = ?4, consecutive_failures = 0, streak_started_at = NULL, changed_at = ?2, reset_fence_at = ?2, last_failure_at = NULL, last_failure_kind = NULL, last_failure_message = NULL, cooldown_until = NULL, cache_concurrency_limit = NULL, cache_recovery_limit = NULL, cache_low_hit_streak = 0, cache_cooldown_level = 0, cache_last_hit_rate_percent = NULL, cache_usage_missing_since = NULL, cache_usage_missing_reason = NULL WHERE account_id = ?1 AND model = ?5",
    )
    .bind(account_id)
    .bind(&now)
    .bind(MODEL_ROUTE_STATE_AVAILABLE)
    .bind(MODEL_ROUTE_PRIORITY_NORMAL)
    .bind(model)
    .execute(pool)
    .await?;
    reset_priority_handoff_for_model(account_id, model);
    if let Err(error) = persist_model_event(
        pool,
        account_id,
        None,
        model,
        UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_RESET,
        "manual",
        "reset",
        Some(&before_state),
        Some(MODEL_ROUTE_STATE_AVAILABLE),
        Some(&before_priority),
        Some(MODEL_ROUTE_PRIORITY_NORMAL),
        0,
        None,
        Some("model route manually reset"),
        None,
        None,
        None,
    )
    .await
    {
        warn!(
            account_id,
            model,
            error = %error,
            "failed to persist manual model route reset event"
        );
    }
    let updated = sqlx::query_as::<_, ModelRouteRow>(
        "SELECT account_id, model, state, priority, consecutive_failures, streak_started_at, changed_at, last_seen_at, last_success_at, last_failure_at, last_failure_kind, last_failure_message, cooldown_until, reset_fence_at, cache_concurrency_limit, cache_recovery_limit, cache_low_hit_streak, cache_cooldown_level, cache_last_hit_rate_percent, cache_usage_missing_since, cache_usage_missing_reason FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .fetch_optional(pool)
    .await?;
    Ok(updated.map(model_state_from_row))
}
