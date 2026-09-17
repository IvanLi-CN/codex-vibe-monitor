pub(crate) struct RuntimePromptCacheStickyRouteRequest<'a> {
    pub(crate) sticky_key: &'a str,
    pub(crate) prompt_cache_key: Option<&'a str>,
    pub(crate) upstream_account_id: i64,
    pub(crate) now_iso: &'a str,
    pub(crate) invoke_id: Option<&'a str>,
    pub(crate) attempt_id: Option<i64>,
    pub(crate) sticky_affinity_generation: Option<i64>,
}

struct RuntimeStickyUpsertContext {
    trigger_attempt_id: Option<String>,
    routing_source: Option<String>,
    routing_selection_audit: Option<PoolRoutingSelectionAudit>,
    trigger_invoke_id: Option<String>,
    request_model: Option<String>,
    current_epoch: i64,
    current_generation: i64,
    model_key: Option<String>,
    pending_clear_cause: (Option<String>, Option<u16>),
    sticky_before: Option<PromptCacheConversationOperationStickySnapshot>,
    routing_scope: PromptCacheConversationOperationRoutingScope,
}

async fn load_runtime_sticky_upsert_context(
    connection: &mut SqliteConnection,
    sticky_key: &str,
    event_prompt_cache_key: Option<&str>,
    attempt_id: Option<i64>,
) -> Result<RuntimeStickyUpsertContext> {
    let (
        trigger_attempt_id,
        routing_source,
        routing_selection_audit,
        trigger_invoke_id,
        request_model,
    ) = load_runtime_attempt_routing_context_executor(&mut *connection, attempt_id).await?;
    let current_epoch =
        load_sticky_affinity_generation_executor(&mut *connection, sticky_key).await?;
    let model_key = normalize_sticky_model_key(request_model.as_deref());
    let pending_clear_cause = load_pending_sticky_clear_cause_executor(
        &mut *connection,
        sticky_key,
        model_key.as_deref(),
    )
    .await?;
    let current_generation = match model_key.as_deref() {
        Some(model_key) => {
            load_sticky_model_generation_executor(&mut *connection, sticky_key, model_key).await?
        }
        None => current_epoch,
    };
    let sticky_before = if event_prompt_cache_key.is_some() {
        load_prompt_cache_conversation_sticky_snapshot_for_model_executor(
            &mut *connection,
            sticky_key,
            model_key.as_deref(),
        )
        .await?
    } else {
        None
    };
    let routing_scope = PromptCacheConversationOperationRoutingScope {
        kind: if model_key.is_some() {
            "model".to_string()
        } else {
            "all".to_string()
        },
        model_key: model_key.clone(),
        request_model: model_key.as_ref().and_then(|model_key| {
            request_model
                .clone()
                .filter(|request_model| request_model != model_key)
        }),
    };
    Ok(RuntimeStickyUpsertContext {
        trigger_attempt_id,
        routing_source,
        routing_selection_audit,
        trigger_invoke_id,
        request_model,
        current_epoch,
        current_generation,
        model_key,
        pending_clear_cause,
        sticky_before,
        routing_scope,
    })
}

fn runtime_sticky_generation_is_stale(
    context: &RuntimeStickyUpsertContext,
    expected_generation: Option<i64>,
) -> bool {
    let Some(expected_generation) = expected_generation else {
        return false;
    };
    if context.model_key.is_some() {
        let (expected_epoch, expected_model_generation) =
            unpack_sticky_affinity_token(expected_generation);
        context.current_epoch != expected_epoch
            || context.current_generation != expected_model_generation
    } else {
        context.current_generation != expected_generation
    }
}

async fn append_sticky_mutation_suppressed_event(
    connection: &mut SqliteConnection,
    prompt_cache_key: Option<&str>,
    now_iso: &str,
    invoke_id: Option<&str>,
    context: &RuntimeStickyUpsertContext,
) -> Result<()> {
    let Some(prompt_cache_key) = prompt_cache_key else {
        return Ok(());
    };
    append_prompt_cache_conversation_operation_event_with_routing_scope_executor(
        &mut *connection,
        AppendPromptCacheConversationOperationEventInput {
            prompt_cache_key: prompt_cache_key.to_string(),
            action: "stickyMutationSuppressed".to_string(),
            origin: PROMPT_CACHE_CONVERSATION_OPERATION_ORIGIN_SYSTEM_AUTO.to_string(),
            info_types: vec![PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_ROUTING.to_string()],
            occurred_at: now_iso.to_string(),
            headline: prompt_cache_conversation_operation_headline("stickyMutationSuppressed"),
            changed_fields: Vec::new(),
            binding_before: None,
            binding_after: None,
            sticky_before: context.sticky_before.clone(),
            sticky_after: context.sticky_before.clone(),
            invoke_id: invoke_id
                .map(ToOwned::to_owned)
                .or_else(|| context.trigger_invoke_id.clone()),
        },
        Some(PromptCacheConversationOperationRoutingContext {
            reason_code: "staleConcurrentCompletion".to_string(),
            routing_source: context.routing_source.clone(),
            routing_selection_audit: context.routing_selection_audit.clone(),
            http_status: None,
            trigger_attempt_id: context.trigger_attempt_id.clone(),
            causing_attempt_id: None,
            causing_http_status: None,
        }),
        context.routing_scope.clone(),
    )
    .await
}

async fn load_previous_sticky_account_id(
    connection: &mut SqliteConnection,
    sticky_key: &str,
    model_key: Option<&str>,
) -> Result<Option<i64>> {
    if let Some(model_key) = model_key {
        return Ok(sqlx::query_scalar::<_, i64>(
            "SELECT account_id FROM pool_sticky_model_routes WHERE sticky_key = ?1 AND model_key = ?2 LIMIT 1",
        )
        .bind(sticky_key)
        .bind(model_key)
        .fetch_optional(&mut *connection)
        .await?);
    }
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT account_id FROM pool_sticky_routes WHERE sticky_key = ?1 LIMIT 1",
    )
    .bind(sticky_key)
    .fetch_optional(&mut *connection)
    .await?)
}

async fn apply_runtime_sticky_route(
    connection: &mut SqliteConnection,
    sticky_key: &str,
    model_key: Option<&str>,
    upstream_account_id: i64,
    now_iso: &str,
    target_changed: bool,
) -> Result<()> {
    if let Some(model_key) = model_key {
        upsert_sticky_model_route_executor(
            &mut *connection,
            sticky_key,
            model_key,
            upstream_account_id,
            now_iso,
        )
        .await?;
        if target_changed {
            bump_sticky_model_generation_executor(&mut *connection, sticky_key, model_key, now_iso)
                .await?;
        }
    } else {
        upsert_sticky_route_executor(&mut *connection, sticky_key, upstream_account_id, now_iso)
            .await?;
        if target_changed {
            bump_sticky_affinity_generation_executor(&mut *connection, sticky_key, now_iso).await?;
        }
    }
    Ok(())
}

async fn append_runtime_sticky_changed_event(
    connection: &mut SqliteConnection,
    prompt_cache_key: &str,
    now_iso: &str,
    invoke_id: Option<&str>,
    context: &RuntimeStickyUpsertContext,
    previous_account_id: Option<i64>,
    sticky_after: PromptCacheConversationOperationStickySnapshot,
) -> Result<()> {
    let (causing_attempt_id, causing_http_status) = if previous_account_id.is_none() {
        context.pending_clear_cause.clone()
    } else {
        (None, None)
    };
    let reason_code = if causing_attempt_id.is_some() {
        "freshAssignmentAfterFailure"
    } else if previous_account_id.is_some() {
        "stickyTargetChanged"
    } else {
        "firstSuccessfulAssignment"
    };
    append_prompt_cache_conversation_operation_event_with_routing_scope_executor(
        &mut *connection,
        AppendPromptCacheConversationOperationEventInput {
            prompt_cache_key: prompt_cache_key.to_string(),
            action: "stickyTargetChanged".to_string(),
            origin: PROMPT_CACHE_CONVERSATION_OPERATION_ORIGIN_SYSTEM_AUTO.to_string(),
            info_types: vec![PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_ROUTING.to_string()],
            occurred_at: now_iso.to_string(),
            headline: prompt_cache_conversation_operation_headline("stickyTargetChanged"),
            changed_fields: vec!["stickyTarget".to_string()],
            binding_before: None,
            binding_after: None,
            sticky_before: context.sticky_before.clone(),
            sticky_after: Some(sticky_after),
            invoke_id: invoke_id
                .map(ToOwned::to_owned)
                .or_else(|| context.trigger_invoke_id.clone()),
        },
        Some(PromptCacheConversationOperationRoutingContext {
            reason_code: reason_code.to_string(),
            routing_source: context.routing_source.clone(),
            routing_selection_audit: context.routing_selection_audit.clone(),
            http_status: None,
            trigger_attempt_id: context.trigger_attempt_id.clone(),
            causing_attempt_id,
            causing_http_status,
        }),
        context.routing_scope.clone(),
    )
    .await
}

async fn append_runtime_sticky_change_if_needed(
    connection: &mut SqliteConnection,
    event_prompt_cache_key: Option<&str>,
    sticky_key: &str,
    now_iso: &str,
    invoke_id: Option<&str>,
    context: &RuntimeStickyUpsertContext,
    previous_account_id: Option<i64>,
) -> Result<()> {
    let Some(prompt_cache_key) = event_prompt_cache_key else {
        return Ok(());
    };
    let sticky_after = load_prompt_cache_conversation_sticky_snapshot_for_model_executor(
        &mut *connection,
        sticky_key,
        context.model_key.as_deref(),
    )
    .await?;
    let sticky_changed = context.sticky_before != sticky_after;
    if sticky_changed && let Some(sticky_after) = sticky_after {
        append_runtime_sticky_changed_event(
            &mut *connection,
            prompt_cache_key,
            now_iso,
            invoke_id,
            context,
            previous_account_id,
            sticky_after,
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn upsert_runtime_prompt_cache_conversation_sticky_route(
    pool: &Pool<Sqlite>,
    request: RuntimePromptCacheStickyRouteRequest<'_>,
) -> Result<RuntimeStickyMutation> {
    let RuntimePromptCacheStickyRouteRequest {
        sticky_key,
        prompt_cache_key,
        upstream_account_id,
        now_iso,
        invoke_id,
        attempt_id,
        sticky_affinity_generation,
    } = request;
    let event_prompt_cache_key = prompt_cache_key.filter(|key| *key == sticky_key);
    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(conn.as_mut())
        .await
        .context("failed to acquire runtime sticky event write lock")?;

    let write_outcome: Result<RuntimeStickyMutation> = async {
        let context = load_runtime_sticky_upsert_context(
            conn.as_mut(),
            sticky_key,
            event_prompt_cache_key,
            attempt_id,
        )
        .await?;
        if runtime_sticky_generation_is_stale(&context, sticky_affinity_generation) {
            append_sticky_mutation_suppressed_event(
                conn.as_mut(),
                event_prompt_cache_key,
                now_iso,
                invoke_id,
                &context,
            )
            .await?;
            return Ok(RuntimeStickyMutation::Suppressed);
        }
        let previous_account_id = load_previous_sticky_account_id(
            conn.as_mut(),
            sticky_key,
            context.model_key.as_deref(),
        )
        .await?;
        let target_changed = previous_account_id != Some(upstream_account_id);
        apply_runtime_sticky_route(
            conn.as_mut(),
            sticky_key,
            context.model_key.as_deref(),
            upstream_account_id,
            now_iso,
            target_changed,
        )
        .await?;

        append_runtime_sticky_change_if_needed(
            conn.as_mut(),
            event_prompt_cache_key,
            sticky_key,
            now_iso,
            invoke_id,
            &context,
            previous_account_id,
        )
        .await?;
        Ok(if target_changed {
            RuntimeStickyMutation::Changed {
                previous_upstream_account_id: previous_account_id,
            }
        } else {
            RuntimeStickyMutation::Unchanged
        })
    }
    .await;

    match write_outcome {
        Ok(applied) => {
            sqlx::query("COMMIT")
                .execute(conn.as_mut())
                .await
                .context("failed to commit runtime sticky event transaction")?;
            Ok(applied)
        }
        Err(error) => {
            let _ = sqlx::query("ROLLBACK").execute(conn.as_mut()).await;
            Err(error)
        }
    }
}

fn prompt_cache_conversation_operation_event_response_from_row(
    row: PromptCacheConversationOperationEventRow,
) -> PromptCacheConversationOperationEventResponse {
    let deserialize_binding =
        |raw: Option<String>| -> Option<PromptCacheConversationOperationBindingSnapshot> {
            raw.and_then(|value| {
                serde_json::from_str::<PromptCacheConversationOperationBindingSnapshot>(&value).ok()
            })
        };
    let deserialize_sticky =
        |raw: Option<String>| -> Option<PromptCacheConversationOperationStickySnapshot> {
            raw.and_then(|value| {
                serde_json::from_str::<PromptCacheConversationOperationStickySnapshot>(&value).ok()
            })
        };

    PromptCacheConversationOperationEventResponse {
        id: row.id,
        prompt_cache_key: row.prompt_cache_key,
        action: row.action,
        origin: row.origin,
        info_types: parse_prompt_cache_conversation_operation_string_array_json(Some(
            row.info_types_json.as_str(),
        )),
        occurred_at: row.occurred_at,
        headline: row.headline,
        changed_fields: parse_prompt_cache_conversation_operation_string_array_json(
            row.changed_fields_json.as_deref(),
        ),
        binding_before: deserialize_binding(row.binding_before_json),
        binding_after: deserialize_binding(row.binding_after_json),
        sticky_before: deserialize_sticky(row.sticky_before_json),
        sticky_after: deserialize_sticky(row.sticky_after_json),
        invoke_id: row.invoke_id,
        routing_context: row.routing_context_json.and_then(|value| {
            serde_json::from_str::<PromptCacheConversationOperationRoutingContext>(&value).ok()
        }),
        routing_scope: row.routing_scope_json.and_then(|value| {
            serde_json::from_str::<PromptCacheConversationOperationRoutingScope>(&value).ok()
        }),
        sticky_transitions: row
            .sticky_transitions_json
            .as_deref()
            .and_then(|value| {
                serde_json::from_str::<Vec<PromptCacheConversationOperationStickyTransition>>(value)
                    .ok()
            })
            .unwrap_or_default(),
    }
}

pub(crate) async fn resolve_effective_forward_proxy_key_for_account(
    state: &AppState,
    account_id: Option<i64>,
) -> Option<String> {
    let account_id = account_id?;
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .ok()??;
    let scope = resolve_account_forward_proxy_scope(state, &row, None)
        .await
        .ok()?;
    forward_proxy_key_for_scope(state, &scope).await
}

pub(crate) async fn load_first_group_account_id_for_forward_proxy(
    pool: &Pool<Sqlite>,
    group_name: &str,
) -> Result<Option<i64>> {
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id
        FROM pool_upstream_accounts
        WHERE TRIM(COALESCE(group_name, '')) = ?1
          AND provider = 'codex'
          AND enabled != 0
          AND status = 'active'
          AND encrypted_credentials IS NOT NULL
        ORDER BY id ASC
        LIMIT 1
        "#,
    )
    .bind(group_name)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub(crate) async fn resolve_effective_forward_proxy_key_for_row(
    state: &AppState,
    row: &PromptCacheConversationBindingRow,
    owner: Option<&PromptCacheEncryptedSessionOwnerRow>,
    sticky_account_id: Option<i64>,
) -> Option<String> {
    if row.binding_kind == PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT {
        return resolve_effective_forward_proxy_key_for_account(state, row.upstream_account_id)
            .await;
    }
    if row.binding_kind == PROMPT_CACHE_BINDING_KIND_GROUP {
        let account_id =
            load_first_group_account_id_for_forward_proxy(&state.pool, row.group_name.as_deref()?)
                .await
                .ok()?;
        return resolve_effective_forward_proxy_key_for_account(state, account_id).await;
    }
    let effective_account_id = owner
        .map(|owner| owner.owner_upstream_account_id)
        .or(sticky_account_id);
    resolve_effective_forward_proxy_key_for_account(state, effective_account_id).await
}

pub(crate) async fn forward_proxy_key_for_scope(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
) -> Option<String> {
    match scope {
        ForwardProxyRouteScope::PinnedProxyKey(proxy_key) => Some(proxy_key.clone()),
        ForwardProxyRouteScope::Automatic
        | ForwardProxyRouteScope::BoundGroup { .. }
        | ForwardProxyRouteScope::BoundProxyKeys { .. } => {
            select_forward_proxy_for_scope(state, scope)
                .await
                .ok()
                .map(|selected| selected.key)
        }
    }
}

pub(crate) fn apply_owner_to_none_response(
    mut response: PromptCacheConversationBindingResponse,
    owner: Option<&PromptCacheEncryptedSessionOwnerRow>,
) -> PromptCacheConversationBindingResponse {
    if let Some(owner) = owner {
        response.has_encrypted_session_owner = true;
        response.encrypted_owner_account_id = Some(owner.owner_upstream_account_id);
        response.encrypted_owner_account_name = owner.owner_upstream_account_name.clone();
        response.encrypted_owner_group_name = owner.owner_group_name.clone();
    }
    response
}

pub(crate) async fn load_prompt_cache_conversation_binding_row_executor<'e, E>(
    executor: E,
    prompt_cache_key: &str,
) -> Result<Option<PromptCacheConversationBindingRow>>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    sqlx::query_as::<_, PromptCacheConversationBindingRow>(
        r#"
        SELECT
            binding.prompt_cache_key,
            binding.binding_kind,
            binding.group_name,
            binding.upstream_account_id,
            account.display_name AS upstream_account_name,
            binding.responses_first_byte_timeout_secs,
            binding.compact_first_byte_timeout_secs,
            binding.image_first_byte_timeout_secs,
            binding.responses_stream_timeout_secs,
            binding.compact_stream_timeout_secs,
            binding.allow_switch_upstream,
            binding.fast_mode_rewrite_mode,
            binding.image_tool_rewrite_mode,
            binding.codex_imagegen_rewrite_mode,
            binding.available_models_json,
            binding.available_models_mode,
            binding.forward_proxy_key,
            binding.forward_proxy_keys_json,
            binding.updated_at
        FROM prompt_cache_conversation_bindings AS binding
        LEFT JOIN pool_upstream_accounts AS account
          ON account.id = binding.upstream_account_id
        WHERE binding.prompt_cache_key = ?1
        LIMIT 1
        "#,
    )
    .bind(prompt_cache_key)
    .fetch_optional(executor)
    .await
    .map_err(Into::into)
}

pub(crate) async fn load_prompt_cache_conversation_binding_row(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
) -> Result<Option<PromptCacheConversationBindingRow>> {
    load_prompt_cache_conversation_binding_row_executor(pool, prompt_cache_key).await
}

pub(crate) fn parse_fast_mode_rewrite_mode_lossy(value: &str) -> TagFastModeRewriteMode {
    match value.trim() {
        "force_remove" => TagFastModeRewriteMode::ForceRemove,
        "fill_missing" => TagFastModeRewriteMode::FillMissing,
        "force_add" => TagFastModeRewriteMode::ForceAdd,
        _ => TagFastModeRewriteMode::KeepOriginal,
    }
}

pub(crate) fn normalize_fast_mode_rewrite_mode(
    value: PatchField<String>,
) -> Result<PatchField<TagFastModeRewriteMode>, ApiError> {
    let value = match value {
        PatchField::Missing => return Ok(PatchField::Missing),
        PatchField::Null => return Ok(PatchField::Null),
        PatchField::Value(value) => value,
    };
    match value.trim() {
        "force_remove" => Ok(PatchField::Value(TagFastModeRewriteMode::ForceRemove)),
        "keep_original" => Ok(PatchField::Value(TagFastModeRewriteMode::KeepOriginal)),
        "fill_missing" => Ok(PatchField::Value(TagFastModeRewriteMode::FillMissing)),
        "force_add" => Ok(PatchField::Value(TagFastModeRewriteMode::ForceAdd)),
        _ => Err(ApiError::bad_request(anyhow!(
            "fastModeRewriteMode must be one of: force_remove, keep_original, fill_missing, force_add"
        ))),
    }
}

pub(crate) fn normalize_image_tool_rewrite_mode(
    value: PatchField<String>,
) -> Result<PatchField<ImageToolRewriteMode>, ApiError> {
    let value = match value {
        PatchField::Missing => return Ok(PatchField::Missing),
        PatchField::Null => return Ok(PatchField::Null),
        PatchField::Value(value) => value,
    };
    match value.trim() {
        "force_remove" => Ok(PatchField::Value(ImageToolRewriteMode::ForceRemove)),
        "keep_original" => Ok(PatchField::Value(ImageToolRewriteMode::KeepOriginal)),
        "fill_missing" => Ok(PatchField::Value(ImageToolRewriteMode::FillMissing)),
        "force_add" => Ok(PatchField::Value(ImageToolRewriteMode::ForceAdd)),
        _ => Err(ApiError::bad_request(anyhow!(
            "imageToolRewriteMode must be one of: force_remove, keep_original, fill_missing, force_add"
        ))),
    }
}

pub(crate) fn normalize_codex_imagegen_rewrite_mode(
    value: PatchField<String>,
) -> Result<PatchField<CodexImagegenRewriteMode>, ApiError> {
    let value = match value {
        PatchField::Missing => return Ok(PatchField::Missing),
        PatchField::Null => return Ok(PatchField::Null),
        PatchField::Value(value) => value,
    };
    match value.trim() {
        "force_remove" => Ok(PatchField::Value(CodexImagegenRewriteMode::ForceRemove)),
        "keep_original" => Ok(PatchField::Value(CodexImagegenRewriteMode::KeepOriginal)),
        "fill_missing" => Ok(PatchField::Value(CodexImagegenRewriteMode::FillMissing)),
        "force_add" => Ok(PatchField::Value(CodexImagegenRewriteMode::ForceAdd)),
        _ => Err(ApiError::bad_request(anyhow!(
            "codexImagegenRewriteMode must be one of: force_remove, keep_original, fill_missing, force_add"
        ))),
    }
}

pub(crate) fn normalize_available_models_patch(
    value: PatchField<Vec<String>>,
) -> Result<PatchField<Vec<String>>, ApiError> {
    let values = match value {
        PatchField::Missing => return Ok(PatchField::Missing),
        PatchField::Null => return Ok(PatchField::Null),
        PatchField::Value(values) => values,
    };
    let mut normalized = Vec::new();
    for value in values {
        let model = value.trim();
        if !model.is_empty() && !normalized.iter().any(|candidate| candidate == model) {
            normalized.push(model.to_string());
        }
    }
    Ok(PatchField::Value(normalized))
}
