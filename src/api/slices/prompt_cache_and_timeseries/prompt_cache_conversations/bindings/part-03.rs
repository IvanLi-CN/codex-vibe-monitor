async fn append_prompt_cache_conversation_operation_event_executor<'e, E>(
    executor: E,
    input: AppendPromptCacheConversationOperationEventInput,
) -> Result<()>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    append_prompt_cache_conversation_operation_event_with_routing_context_executor(
        executor, input, None,
    )
    .await
}

async fn append_prompt_cache_conversation_operation_event_with_routing_context_executor<'e, E>(
    executor: E,
    input: AppendPromptCacheConversationOperationEventInput,
    routing_context: Option<PromptCacheConversationOperationRoutingContext>,
) -> Result<()>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let routing_scope = input
        .info_types
        .iter()
        .any(|value| value == PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_ROUTING)
        .then(|| {
            serde_json::to_string(&PromptCacheConversationOperationRoutingScope {
                kind: "all".to_string(),
                model_key: None,
                request_model: None,
            })
        })
        .transpose()?;
    sqlx::query(
        r#"
        INSERT INTO prompt_cache_conversation_operation_events (
            prompt_cache_key,
            action,
            origin,
            info_types_json,
            occurred_at,
            headline,
            changed_fields_json,
            binding_before_json,
            binding_after_json,
            sticky_before_json,
            sticky_after_json,
            invoke_id,
            routing_context_json,
            routing_scope_json
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
        "#,
    )
    .bind(input.prompt_cache_key)
    .bind(input.action)
    .bind(input.origin)
    .bind(serde_json::to_string(&input.info_types)?)
    .bind(input.occurred_at)
    .bind(input.headline)
    .bind(
        (!input.changed_fields.is_empty())
            .then(|| serde_json::to_string(&input.changed_fields))
            .transpose()?,
    )
    .bind(
        input
            .binding_before
            .map(|value| serde_json::to_string(&value))
            .transpose()?,
    )
    .bind(
        input
            .binding_after
            .map(|value| serde_json::to_string(&value))
            .transpose()?,
    )
    .bind(
        input
            .sticky_before
            .map(|value| serde_json::to_string(&value))
            .transpose()?,
    )
    .bind(
        input
            .sticky_after
            .map(|value| serde_json::to_string(&value))
            .transpose()?,
    )
    .bind(input.invoke_id)
    .bind(
        routing_context
            .map(|value| serde_json::to_string(&value))
            .transpose()?,
    )
    .bind(routing_scope)
    .execute(executor)
    .await?;
    Ok(())
}

async fn append_prompt_cache_conversation_operation_event_with_routing_scope_executor<'e, E>(
    executor: E,
    input: AppendPromptCacheConversationOperationEventInput,
    routing_context: Option<PromptCacheConversationOperationRoutingContext>,
    routing_scope: PromptCacheConversationOperationRoutingScope,
) -> Result<()>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        r#"
        INSERT INTO prompt_cache_conversation_operation_events (
            prompt_cache_key,
            action,
            origin,
            info_types_json,
            occurred_at,
            headline,
            changed_fields_json,
            binding_before_json,
            binding_after_json,
            sticky_before_json,
            sticky_after_json,
            invoke_id,
            routing_context_json,
            routing_scope_json
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
        "#,
    )
    .bind(input.prompt_cache_key)
    .bind(input.action)
    .bind(input.origin)
    .bind(serde_json::to_string(&input.info_types)?)
    .bind(input.occurred_at)
    .bind(input.headline)
    .bind(
        (!input.changed_fields.is_empty())
            .then(|| serde_json::to_string(&input.changed_fields))
            .transpose()?,
    )
    .bind(
        input
            .binding_before
            .map(|value| serde_json::to_string(&value))
            .transpose()?,
    )
    .bind(
        input
            .binding_after
            .map(|value| serde_json::to_string(&value))
            .transpose()?,
    )
    .bind(
        input
            .sticky_before
            .map(|value| serde_json::to_string(&value))
            .transpose()?,
    )
    .bind(
        input
            .sticky_after
            .map(|value| serde_json::to_string(&value))
            .transpose()?,
    )
    .bind(input.invoke_id)
    .bind(
        routing_context
            .map(|value| serde_json::to_string(&value))
            .transpose()?,
    )
    .bind(serde_json::to_string(&routing_scope)?)
    .execute(executor)
    .await?;
    Ok(())
}

async fn append_prompt_cache_conversation_operation_event_with_sticky_transitions_executor<'e, E>(
    executor: E,
    input: AppendPromptCacheConversationOperationEventInput,
    sticky_transitions: &[PromptCacheConversationOperationStickyTransition],
) -> Result<()>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let routing_scope = PromptCacheConversationOperationRoutingScope {
        kind: "all".to_string(),
        model_key: None,
        request_model: None,
    };
    sqlx::query(
        r#"
        INSERT INTO prompt_cache_conversation_operation_events (
            prompt_cache_key,
            action,
            origin,
            info_types_json,
            occurred_at,
            headline,
            changed_fields_json,
            binding_before_json,
            binding_after_json,
            sticky_before_json,
            sticky_after_json,
            invoke_id,
            routing_scope_json,
            sticky_transitions_json
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
        "#,
    )
    .bind(input.prompt_cache_key)
    .bind(input.action)
    .bind(input.origin)
    .bind(serde_json::to_string(&input.info_types)?)
    .bind(input.occurred_at)
    .bind(input.headline)
    .bind(
        (!input.changed_fields.is_empty())
            .then(|| serde_json::to_string(&input.changed_fields))
            .transpose()?,
    )
    .bind(
        input
            .binding_before
            .map(|value| serde_json::to_string(&value))
            .transpose()?,
    )
    .bind(
        input
            .binding_after
            .map(|value| serde_json::to_string(&value))
            .transpose()?,
    )
    .bind(
        input
            .sticky_before
            .map(|value| serde_json::to_string(&value))
            .transpose()?,
    )
    .bind(
        input
            .sticky_after
            .map(|value| serde_json::to_string(&value))
            .transpose()?,
    )
    .bind(input.invoke_id)
    .bind(serde_json::to_string(&routing_scope)?)
    .bind(serde_json::to_string(sticky_transitions)?)
    .execute(executor)
    .await?;
    Ok(())
}

fn prompt_cache_conversation_sticky_transitions(
    before: &[PromptCacheConversationStickyRouteResponse],
    after: &[PromptCacheConversationStickyRouteResponse],
) -> Vec<PromptCacheConversationOperationStickyTransition> {
    fn snapshots(
        routes: &[PromptCacheConversationStickyRouteResponse],
    ) -> BTreeMap<Option<String>, PromptCacheConversationOperationStickySnapshot> {
        routes
            .iter()
            .map(|route| {
                (
                    route.model_key.clone(),
                    PromptCacheConversationOperationStickySnapshot {
                        upstream_account_id: route.upstream_account_id,
                        upstream_account_name: route.upstream_account_name.clone(),
                    },
                )
            })
            .collect()
    }

    let before = snapshots(before);
    let after = snapshots(after);
    let mut model_keys = before
        .keys()
        .chain(after.keys())
        .cloned()
        .collect::<Vec<_>>();
    model_keys.sort();
    model_keys.dedup();
    model_keys
        .into_iter()
        .filter_map(|model_key| {
            let before = before.get(&model_key).cloned();
            let after = after.get(&model_key).cloned();
            (before != after).then_some(PromptCacheConversationOperationStickyTransition {
                model_key,
                before,
                after,
            })
        })
        .collect()
}

pub(crate) struct RuntimeStickyTargetClearedEvent {
    pub(crate) prompt_cache_key: String,
    pub(crate) account_id: i64,
    pub(crate) account_name: Option<String>,
    pub(crate) occurred_at: String,
    pub(crate) invoke_id: Option<String>,
    pub(crate) routing_context: PromptCacheConversationOperationRoutingContext,
    pub(crate) routing_scope: PromptCacheConversationOperationRoutingScope,
}

pub(crate) async fn append_runtime_sticky_target_cleared_event_executor<'e, E>(
    executor: E,
    event: RuntimeStickyTargetClearedEvent,
) -> Result<()>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let RuntimeStickyTargetClearedEvent {
        prompt_cache_key,
        account_id,
        account_name,
        occurred_at,
        invoke_id,
        routing_context,
        routing_scope,
    } = event;
    let sticky_before = PromptCacheConversationOperationStickySnapshot {
        upstream_account_id: account_id,
        upstream_account_name: account_name,
    };
    sqlx::query(
        r#"
        INSERT INTO prompt_cache_conversation_operation_events (
            prompt_cache_key,
            action,
            origin,
            info_types_json,
            occurred_at,
            headline,
            changed_fields_json,
            sticky_before_json,
            invoke_id,
            routing_context_json,
            routing_scope_json
        )
        VALUES (?1, 'stickyTargetCleared', 'systemAuto', '["routing"]', ?2,
                'Sticky target cleared', '["stickyTarget"]', ?3, ?4, ?5, ?6)
        "#,
    )
    .bind(prompt_cache_key)
    .bind(occurred_at)
    .bind(serde_json::to_string(&sticky_before)?)
    .bind(invoke_id)
    .bind(serde_json::to_string(&routing_context)?)
    .bind(serde_json::to_string(&routing_scope)?)
    .execute(executor)
    .await?;
    Ok(())
}

async fn load_runtime_attempt_routing_context_executor<'e, E>(
    executor: E,
    attempt_id: Option<i64>,
) -> Result<(
    Option<String>,
    Option<String>,
    Option<PoolRoutingSelectionAudit>,
    Option<String>,
    Option<String>,
)>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let Some(attempt_id) = attempt_id else {
        return Ok((None, None, None, None, None));
    };
    sqlx::query_as::<_, (Option<String>, Option<String>, Option<String>, Option<String>, Option<String>)>(
        "SELECT attempt_public_id, routing_source, routing_selection_audit_json, invoke_id, request_model FROM pool_upstream_request_attempts WHERE id = ?1",
    )
    .bind(attempt_id)
    .fetch_optional(executor)
    .await
    .map(|value| {
        value
            .map(|(attempt_public_id, routing_source, routing_selection_audit_json, invoke_id, request_model)| {
                (
                    attempt_public_id,
                    routing_source,
                    routing_selection_audit_json
                        .as_deref()
                        .and_then(|raw| serde_json::from_str(raw).ok()),
                    invoke_id,
                    request_model,
                )
            })
            .unwrap_or((None, None, None, None, None))
    })
    .map_err(Into::into)
}

async fn load_pending_sticky_clear_cause_executor<'e, E>(
    executor: E,
    sticky_key: &str,
    model_key: Option<&str>,
) -> Result<(Option<String>, Option<u16>)>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let row = if let Some(model_key) = model_key {
        sqlx::query_as::<_, (Option<String>, Option<i64>)>(
            r#"
            SELECT last_clear_cause_attempt_public_id, last_clear_cause_http_status
            FROM pool_sticky_model_route_generations
            WHERE sticky_key = ?1 AND model_key = ?2
            LIMIT 1
            "#,
        )
        .bind(sticky_key)
        .bind(model_key)
        .fetch_optional(executor)
        .await?
    } else {
        sqlx::query_as::<_, (Option<String>, Option<i64>)>(
            r#"
            SELECT last_clear_cause_attempt_public_id, last_clear_cause_http_status
            FROM pool_sticky_route_generations
            WHERE sticky_key = ?1
            LIMIT 1
            "#,
        )
        .bind(sticky_key)
        .fetch_optional(executor)
        .await?
    };
    Ok(row
        .map(|(attempt_id, status)| {
            (
                attempt_id,
                status.and_then(|value| u16::try_from(value).ok()),
            )
        })
        .unwrap_or((None, None)))
}

async fn append_prompt_cache_conversation_operation_event(
    pool: &Pool<Sqlite>,
    input: AppendPromptCacheConversationOperationEventInput,
) -> Result<()> {
    append_prompt_cache_conversation_operation_event_executor(pool, input).await
}

pub(crate) async fn broadcast_prompt_cache_conversation_changed(
    state: &AppState,
    prompt_cache_key: &str,
) {
    let runtime_cache = state.pool_routing_runtime_cache.lock().await;
    if let Some(runtime_cache) = runtime_cache.as_ref()
        && let Ok(mut cache) = runtime_cache.prompt_route_cache.lock()
    {
        cache.invalidate_prompt_cache_key(prompt_cache_key);
        if let Ok(mut sticky_cache) = runtime_cache.sticky_route_cache.lock() {
            sticky_cache.invalidate_sticky_key(prompt_cache_key);
        }
    }
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::PromptCacheBindingChanged {
            prompt_cache_key: prompt_cache_key.to_string(),
        });
    #[cfg(test)]
    {
        let _ = state
            .broadcaster
            .send(BroadcastPayload::PromptCacheConversationChanged {
                prompt_cache_key: prompt_cache_key.to_string(),
            });
    }
}

pub(crate) async fn invalidate_pool_routing_sticky_route_cache(state: &AppState, sticky_key: &str) {
    let runtime_cache = state.pool_routing_runtime_cache.lock().await;
    if let Some(runtime_cache) = runtime_cache.as_ref()
        && let Ok(mut cache) = runtime_cache.sticky_route_cache.lock()
    {
        cache.invalidate_sticky_key(sticky_key);
    }
}

pub(crate) async fn broadcast_prompt_cache_conversation_sticky_route_changed(
    state: &AppState,
    sticky_key: &str,
    previous_upstream_account_id: i64,
    upstream_account_id: i64,
) {
    invalidate_pool_routing_sticky_route_cache(state, sticky_key).await;
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::StickyRouteChanged {
            sticky_key: sticky_key.to_string(),
            previous_upstream_account_id,
            upstream_account_id,
        });
    #[cfg(test)]
    {
        let _ = state.broadcaster.send(
            BroadcastPayload::PromptCacheConversationStickyRouteChanged {
                sticky_key: sticky_key.to_string(),
                previous_upstream_account_id,
                upstream_account_id,
            },
        );
    }
}
