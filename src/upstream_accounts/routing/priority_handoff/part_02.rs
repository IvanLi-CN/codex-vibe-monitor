async fn complete_priority_handoff_from_attempt_inner(
    pool: &Pool<Sqlite>,
    attempt_id: Option<i64>,
    success: bool,
    cooldown: bool,
    defer_failure: bool,
    model_route_recovered: Option<bool>,
    persist_admitted: bool,
) {
    let Some(attempt_id) = attempt_id else {
        return;
    };
    if success
        && !prepare_successful_priority_handoff_attempt(
            pool,
            attempt_id,
            cooldown,
            defer_failure,
            persist_admitted,
        )
        .await
    {
        return;
    }
    if let Some(context) = take_priority_handoff_attempt(attempt_id) {
        complete_taken_priority_handoff_attempt(
            pool,
            attempt_id,
            context,
            success,
            cooldown,
            model_route_recovered,
            persist_admitted,
        )
        .await;
        return;
    }
    let Some((account_id, model_key, generation)) =
        load_priority_handoff_failure_context(pool, attempt_id).await
    else {
        return;
    };
    defer_priority_handoff_failure_for_key(account_id, model_key.as_str(), generation, cooldown);
}

async fn complete_taken_priority_handoff_attempt(
    pool: &Pool<Sqlite>,
    attempt_id: i64,
    context: PriorityHandoffAttemptContext,
    success: bool,
    cooldown: bool,
    model_route_recovered: Option<bool>,
    persist_admitted: bool,
) {
    let reason_code = if success {
        if model_route_recovered == Some(false) {
            release_priority_handoff_for_key(
                context.account_id,
                &context.model_key,
                context.generation,
            );
            return;
        }
        complete_priority_handoff_for_request(
            context.account_id,
            Some(context.model_key.as_str()),
            Some(context.generation),
            true,
            false,
        )
    } else if cooldown {
        complete_failure_for_key(
            context.account_id,
            &context.model_key,
            context.generation,
            cooldown,
        )
    } else {
        defer_priority_handoff_failure_for_key(
            context.account_id,
            context.model_key.as_str(),
            context.generation,
            cooldown,
        );
        None
    };
    if let Some(reason_code) = reason_code
        && let Err(error) = persist_priority_handoff_event_for_completion(
            pool,
            context.account_id,
            Some(attempt_id),
            context.model_key.as_str(),
            reason_code,
            persist_admitted,
        )
        .await
    {
        warn!(
            account_id = context.account_id,
            attempt_id,
            error = %error,
            reason_code,
            "failed to persist priority handoff event"
        );
    }
}

async fn load_priority_handoff_failure_context(
    pool: &Pool<Sqlite>,
    attempt_id: i64,
) -> Option<(i64, String, u64)> {
    let Some((
        Some(source),
        model,
        downstream_http_status,
        attempt_status,
        failure_kind,
        routing_selection_audit_json,
        Some(account_id),
    )) = sqlx::query_as::<_, (
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
    )>(
        "SELECT routing_source, request_model, downstream_http_status, status, failure_kind, routing_selection_audit_json, upstream_account_id FROM pool_upstream_request_attempts WHERE id = ?1",
    )
    .bind(attempt_id)
    .fetch_optional(pool)
    .await
    .ok()?
    else {
        return None;
    };
    if source != PRIORITY_HANDOFF_ROUTING_SOURCE
        || (attempt_status.as_deref() == Some(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS)
            && downstream_http_status.is_some()
            && failure_kind.is_none())
    {
        return None;
    }
    let model_key = normalize_model_key(model.as_deref())?;
    let generation =
        priority_handoff_generation_from_audit_json(routing_selection_audit_json.as_deref())?;
    Some((account_id, model_key, generation))
}

pub(crate) async fn complete_priority_handoff_from_attempt_or_invoke(
    pool: &Pool<Sqlite>,
    attempt_id: Option<i64>,
    invoke_id: Option<&str>,
    success: bool,
    cooldown: bool,
) {
    complete_priority_handoff_from_attempt_or_invoke_inner(
        pool, attempt_id, invoke_id, success, cooldown, None, false,
    )
    .await;
}

pub(crate) async fn complete_priority_handoff_from_attempt_or_invoke_with_model_recovery(
    pool: &Pool<Sqlite>,
    attempt_id: Option<i64>,
    invoke_id: Option<&str>,
    success: bool,
    cooldown: bool,
    model_route_recovered: Option<bool>,
) {
    complete_priority_handoff_from_attempt_or_invoke_inner(
        pool,
        attempt_id,
        invoke_id,
        success,
        cooldown,
        model_route_recovered,
        false,
    )
    .await;
}

pub(crate) async fn complete_priority_handoff_from_attempt_or_invoke_admitted(
    pool: &Pool<Sqlite>,
    attempt_id: Option<i64>,
    invoke_id: Option<&str>,
    success: bool,
    cooldown: bool,
) {
    complete_priority_handoff_from_attempt_or_invoke_inner(
        pool, attempt_id, invoke_id, success, cooldown, None, true,
    )
    .await;
}

async fn persist_priority_handoff_event_for_completion(
    pool: &Pool<Sqlite>,
    account_id: i64,
    attempt_id: Option<i64>,
    model: &str,
    reason_code: &str,
    admitted: bool,
) -> Result<()> {
    if admitted {
        super::model_health::persist_priority_handoff_event_admitted(
            pool,
            account_id,
            attempt_id,
            model,
            reason_code,
        )
        .await
    } else {
        super::model_health::persist_priority_handoff_event(
            pool,
            account_id,
            attempt_id,
            model,
            reason_code,
        )
        .await
    }
}

async fn complete_priority_handoff_from_attempt_or_invoke_inner(
    pool: &Pool<Sqlite>,
    attempt_id: Option<i64>,
    invoke_id: Option<&str>,
    success: bool,
    cooldown: bool,
    model_route_recovered: Option<bool>,
    persist_admitted: bool,
) {
    if let Some(attempt_id) = attempt_id {
        complete_priority_handoff_from_attempt_inner(
            pool,
            Some(attempt_id),
            success,
            cooldown,
            true,
            model_route_recovered,
            persist_admitted,
        )
        .await;
        return;
    }
    let Some(invoke_id) = invoke_id.filter(|value| !value.is_empty()) else {
        return;
    };
    let Some(context) = take_priority_handoff_attempt_for_invoke(invoke_id) else {
        return;
    };
    if success {
        if model_route_recovered == Some(false) {
            release_priority_handoff_for_key(
                context.account_id,
                &context.model_key,
                context.generation,
            );
            return;
        }
        let reason_code = complete_priority_handoff_for_request(
            context.account_id,
            Some(context.model_key.as_str()),
            Some(context.generation),
            true,
            false,
        );
        if let Some(reason_code) = reason_code
            && let Err(error) = persist_priority_handoff_event_for_completion(
                pool,
                context.account_id,
                None,
                context.model_key.as_str(),
                reason_code,
                persist_admitted,
            )
            .await
        {
            warn!(
                account_id = context.account_id,
                error = %error,
                reason_code,
                "failed to persist priority handoff event"
            );
        }
    } else {
        defer_priority_handoff_failure_for_key(
            context.account_id,
            context.model_key.as_str(),
            context.generation,
            cooldown,
        );
    }
}
