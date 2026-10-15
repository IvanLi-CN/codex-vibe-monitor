use super::*;

pub(super) async fn materialize(
    conn: &mut SqliteConnection,
    event: &RuntimeRouteEvent,
    account: &UpstreamAccountRow,
    model_applied: bool,
) -> Result<()> {
    if let Some(action) = event.action.as_deref() {
        let message = event.message().and_then(sanitize_account_action_message);
        let result = derive_upstream_account_action_result(
            action,
            event.reason_code.as_deref(),
            message.as_deref(),
        );
        sqlx::query("INSERT OR IGNORE INTO pool_upstream_account_events (runtime_event_key,account_id,occurred_at,action,source,account_display_name,account_group_name,result,result_description,reason_code,reason_message,http_status,failure_kind,invoke_id,sticky_key,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?9,?11,?12,?13,?14,?3)")
            .bind(format!("{}:account", event.event_id)).bind(event.account_id).bind(&event.observed_at).bind(action)
            .bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL).bind(&account.display_name).bind(&account.group_name).bind(result).bind(&message)
            .bind(&event.reason_code).bind(event.http_status).bind(event.health_after.as_ref().and_then(|health| health.last_route_failure_kind.as_deref()))
            .bind(&event.invoke_id).bind(event.sticky.as_ref().map(|sticky| &sticky.sticky_key)).execute(&mut *conn).await?;
        if event.update_latest_action {
            sqlx::query("UPDATE pool_upstream_accounts SET last_action=?2,last_action_source=?3,last_action_reason_code=?4,last_action_reason_message=?5,last_action_http_status=?6,last_action_invoke_id=?7,last_action_at=?8 WHERE id=?1 AND (last_action_at IS NULL OR julianday(last_action_at)<=julianday(?8))")
            .bind(event.account_id).bind(action).bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL).bind(&event.reason_code).bind(&message)
            .bind(event.http_status).bind(&event.invoke_id).bind(&event.observed_at).execute(&mut *conn).await?;
        }
    }
    if let Some(fact) = event.model.as_ref().filter(|_| model_applied) {
        let after = &fact.after;
        let before = fact.before.as_ref();
        let changed = before.is_none_or(|before| {
            before.state != after.state
                || before.priority != after.priority
                || before.cooldown_until != after.cooldown_until
                || before.cache_concurrency_limit != after.cache_concurrency_limit
                || before.cache_usage_missing_since != after.cache_usage_missing_since
        });
        if changed && (before.is_some() || after.state != MODEL_ROUTE_STATE_AVAILABLE) {
            let action = if after.state == MODEL_ROUTE_STATE_AVAILABLE {
                UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_RECOVERED
            } else if after.state == MODEL_ROUTE_STATE_COOLING_DOWN {
                UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_COOLDOWN
            } else {
                UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_DEGRADED
            };
            sqlx::query("INSERT OR IGNORE INTO pool_upstream_account_events (runtime_event_key,account_id,occurred_at,action,source,result,reason_code,reason_message,http_status,failure_kind,invoke_id,model,model_route_state_before,model_route_state_after,model_route_priority_before,model_route_priority_after,model_route_failure_count,model_route_cooldown_until,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?3)")
                .bind(format!("{}:model",event.event_id)).bind(event.account_id).bind(&event.observed_at).bind(action).bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL)
                .bind(if after.state == MODEL_ROUTE_STATE_AVAILABLE { "recovered" } else { "failed" }).bind(&event.reason_code).bind(event.http_status)
                .bind(&after.last_failure_kind).bind(&event.invoke_id).bind(&after.model).bind(before.map(|row| &row.state)).bind(&after.state)
                .bind(before.map(|row| &row.priority)).bind(&after.priority).bind(after.consecutive_failures).bind(&after.cooldown_until).execute(&mut *conn).await?;
        }
    }
    Ok(())
}
