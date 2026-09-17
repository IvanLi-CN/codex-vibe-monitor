fn existing_binding_request(
    existing_row: Option<&PromptCacheConversationBindingRow>,
) -> UpdatePromptCacheConversationBindingRequest {
    match existing_row.map(|row| row.binding_kind.as_str()) {
        Some(PROMPT_CACHE_BINDING_KIND_GROUP) => UpdatePromptCacheConversationBindingRequest {
            binding_kind: "group".to_string(),
            group_name: existing_row.and_then(|row| row.group_name.clone()),
            upstream_account_id: None,
            timeouts: None,
            allow_switch_upstream: PatchField::Missing,
            fast_mode_rewrite_mode: PatchField::Missing,
            image_tool_rewrite_mode: PatchField::Missing,
            codex_imagegen_rewrite_mode: PatchField::Missing,
            available_models: PatchField::Missing,
            available_models_mode: PatchField::Missing,
            forward_proxy_key: PatchField::Missing,
            forward_proxy_keys: PatchField::Missing,
        },
        Some(PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT) => {
            UpdatePromptCacheConversationBindingRequest {
                binding_kind: "upstreamAccount".to_string(),
                group_name: None,
                upstream_account_id: existing_row.and_then(|row| row.upstream_account_id),
                timeouts: None,
                allow_switch_upstream: PatchField::Missing,
                fast_mode_rewrite_mode: PatchField::Missing,
                image_tool_rewrite_mode: PatchField::Missing,
                codex_imagegen_rewrite_mode: PatchField::Missing,
                available_models: PatchField::Missing,
                available_models_mode: PatchField::Missing,
                forward_proxy_key: PatchField::Missing,
                forward_proxy_keys: PatchField::Missing,
            }
        }
        _ => UpdatePromptCacheConversationBindingRequest {
            binding_kind: "none".to_string(),
            group_name: None,
            upstream_account_id: None,
            timeouts: None,
            allow_switch_upstream: PatchField::Missing,
            fast_mode_rewrite_mode: PatchField::Missing,
            image_tool_rewrite_mode: PatchField::Missing,
            codex_imagegen_rewrite_mode: PatchField::Missing,
            available_models: PatchField::Missing,
            available_models_mode: PatchField::Missing,
            forward_proxy_key: PatchField::Missing,
            forward_proxy_keys: PatchField::Missing,
        },
    }
}

struct NormalizedPromptCacheBindingUpdate {
    binding_kind: String,
    group_name: Option<String>,
    upstream_account_id: Option<i64>,
    responses_first_byte_timeout_secs: Option<Option<i64>>,
    compact_first_byte_timeout_secs: Option<Option<i64>>,
    image_first_byte_timeout_secs: Option<Option<i64>>,
    responses_stream_timeout_secs: Option<Option<i64>>,
    compact_stream_timeout_secs: Option<Option<i64>>,
    allow_switch_upstream: PatchField<i64>,
    fast_mode_rewrite_mode: PatchField<String>,
    image_tool_rewrite_mode: PatchField<String>,
    codex_imagegen_rewrite_mode: PatchField<String>,
    available_models: PatchField<String>,
    available_models_mode: PatchField<String>,
    forward_proxy_key: PatchField<String>,
    forward_proxy_keys: PatchField<Vec<String>>,
}

struct PromptCacheBindingWritePlan {
    binding_kind: String,
    group_name: Option<String>,
    upstream_account_id: Option<i64>,
    responses_first_byte_timeout_secs: Option<i64>,
    compact_first_byte_timeout_secs: Option<i64>,
    image_first_byte_timeout_secs: Option<i64>,
    responses_stream_timeout_secs: Option<i64>,
    compact_stream_timeout_secs: Option<i64>,
    allow_switch_upstream: Option<i64>,
    fast_mode_rewrite_mode: Option<String>,
    image_tool_rewrite_mode: Option<String>,
    codex_imagegen_rewrite_mode: Option<String>,
    available_models: Option<String>,
    available_models_mode: Option<String>,
    forward_proxy_key: Option<String>,
    forward_proxy_keys_json: Option<String>,
}

async fn normalize_binding_update(
    state: &AppState,
    payload: UpdatePromptCacheConversationBindingRequest,
) -> Result<NormalizedPromptCacheBindingUpdate, ApiError> {
    let binding_kind = payload.binding_kind.trim().to_string();
    let group_name = payload
        .group_name
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if group_name.is_some() && payload.upstream_account_id.is_some() {
        return Err(ApiError::bad_request(anyhow!(
            "groupName and upstreamAccountId are mutually exclusive"
        )));
    }
    let timeout_patch = payload.timeouts.unwrap_or_default();
    let timeout = |value, field: &str| {
        normalize_optional_timeout_override_secs(value, field)
            .map_err(|(_, message)| ApiError::bad_request(anyhow!(message)))
    };
    let available_models = match normalize_available_models_patch(payload.available_models)? {
        PatchField::Missing => PatchField::Missing,
        PatchField::Null => PatchField::Null,
        PatchField::Value(models) => PatchField::Value(serde_json::to_string(&models)?),
    };
    let available_models_mode = match payload.available_models_mode {
        PatchField::Missing if matches!(&available_models, PatchField::Value(_)) => {
            PatchField::Value("allowlist".to_string())
        }
        PatchField::Missing if matches!(&available_models, PatchField::Null) => PatchField::Null,
        PatchField::Missing => PatchField::Missing,
        PatchField::Null => PatchField::Null,
        PatchField::Value(value) => {
            let normalized = value.trim().to_ascii_lowercase();
            if normalized != "allowlist" && normalized != "denylist" {
                return Err(ApiError::bad_request(anyhow!(
                    "availableModelsMode must be allowlist or denylist"
                )));
            }
            PatchField::Value(normalized)
        }
    };
    let forward_proxy_key =
        normalize_forward_proxy_key_patch(state, payload.forward_proxy_key).await?;
    let forward_proxy_keys =
        match normalize_forward_proxy_keys_patch(state, payload.forward_proxy_keys).await? {
            PatchField::Missing => match &forward_proxy_key {
                PatchField::Missing => PatchField::Missing,
                PatchField::Null => PatchField::Null,
                PatchField::Value(value) => PatchField::Value(vec![value.clone()]),
            },
            value => value,
        };
    Ok(NormalizedPromptCacheBindingUpdate {
        binding_kind,
        group_name,
        upstream_account_id: payload.upstream_account_id,
        responses_first_byte_timeout_secs: timeout(
            &timeout_patch.responses_first_byte_timeout_secs,
            "responsesFirstByteTimeoutSecs",
        )?,
        compact_first_byte_timeout_secs: timeout(
            &timeout_patch.compact_first_byte_timeout_secs,
            "compactFirstByteTimeoutSecs",
        )?,
        image_first_byte_timeout_secs: timeout(
            &timeout_patch.image_first_byte_timeout_secs,
            "imageFirstByteTimeoutSecs",
        )?,
        responses_stream_timeout_secs: timeout(
            &timeout_patch.responses_stream_timeout_secs,
            "responsesStreamTimeoutSecs",
        )?,
        compact_stream_timeout_secs: timeout(
            &timeout_patch.compact_stream_timeout_secs,
            "compactStreamTimeoutSecs",
        )?,
        allow_switch_upstream: payload
            .allow_switch_upstream
            .map(|enabled| if enabled { 1 } else { 0 }),
        fast_mode_rewrite_mode: normalize_fast_mode_rewrite_mode(payload.fast_mode_rewrite_mode)?
            .map(|mode| mode.as_str().to_string()),
        image_tool_rewrite_mode: normalize_image_tool_rewrite_mode(
            payload.image_tool_rewrite_mode,
        )?
        .map(|mode| mode.as_str().to_string()),
        codex_imagegen_rewrite_mode: normalize_codex_imagegen_rewrite_mode(
            payload.codex_imagegen_rewrite_mode,
        )?
        .map(|mode| mode.as_str().to_string()),
        available_models,
        available_models_mode,
        forward_proxy_key,
        forward_proxy_keys,
    })
}

fn build_binding_write_plan(
    existing_row: Option<&PromptCacheConversationBindingRow>,
    update: NormalizedPromptCacheBindingUpdate,
) -> Result<PromptCacheBindingWritePlan> {
    let next_timeout = |value, selector: fn(&PromptCacheConversationBindingRow) -> Option<i64>| {
        next_optional_value(value, existing_row.and_then(selector))
    };
    let next_allow_switch_upstream = next_optional_patch_value(
        update.allow_switch_upstream,
        existing_row.and_then(|row| row.allow_switch_upstream),
    );
    let next_fast_mode_rewrite_mode = next_optional_patch_value(
        update.fast_mode_rewrite_mode,
        existing_row.and_then(|row| row.fast_mode_rewrite_mode.clone()),
    );
    let next_image_tool_rewrite_mode = next_optional_patch_value(
        update.image_tool_rewrite_mode,
        existing_row.and_then(|row| row.image_tool_rewrite_mode.clone()),
    );
    let next_codex_imagegen_rewrite_mode = next_optional_patch_value(
        update.codex_imagegen_rewrite_mode,
        existing_row.and_then(|row| row.codex_imagegen_rewrite_mode.clone()),
    );
    let next_available_models = if matches!(&update.available_models_mode, PatchField::Null) {
        None
    } else {
        next_optional_patch_value(
            update.available_models,
            existing_row.and_then(|row| row.available_models_json.clone()),
        )
    };
    let next_available_models_mode = next_optional_patch_value(
        update.available_models_mode,
        existing_row
            .and_then(|row| row.available_models_mode.clone())
            .or_else(|| {
                existing_row.and_then(|row| {
                    row.available_models_json
                        .as_ref()
                        .map(|_| "allowlist".to_string())
                })
            }),
    );
    let next_forward_proxy_keys = next_optional_patch_value(
        update.forward_proxy_keys,
        existing_row
            .map(|row| parse_forward_proxy_keys_json(row.forward_proxy_keys_json.as_deref()))
            .filter(|values| !values.is_empty()),
    );
    let next_forward_proxy_keys_json = next_forward_proxy_keys
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let next_forward_proxy_key = next_forward_proxy_keys
        .as_ref()
        .and_then(|values| values.first().cloned())
        .or_else(|| {
            next_optional_patch_value(
                update.forward_proxy_key,
                existing_row.and_then(|row| row.forward_proxy_key.clone()),
            )
        });
    Ok(PromptCacheBindingWritePlan {
        binding_kind: update.binding_kind,
        group_name: update.group_name,
        upstream_account_id: update.upstream_account_id,
        responses_first_byte_timeout_secs: next_timeout(
            update.responses_first_byte_timeout_secs,
            |row| row.responses_first_byte_timeout_secs,
        ),
        compact_first_byte_timeout_secs: next_timeout(
            update.compact_first_byte_timeout_secs,
            |row| row.compact_first_byte_timeout_secs,
        ),
        image_first_byte_timeout_secs: next_timeout(update.image_first_byte_timeout_secs, |row| {
            row.image_first_byte_timeout_secs
        }),
        responses_stream_timeout_secs: next_timeout(update.responses_stream_timeout_secs, |row| {
            row.responses_stream_timeout_secs
        }),
        compact_stream_timeout_secs: next_timeout(update.compact_stream_timeout_secs, |row| {
            row.compact_stream_timeout_secs
        }),
        allow_switch_upstream: next_allow_switch_upstream,
        fast_mode_rewrite_mode: next_fast_mode_rewrite_mode,
        image_tool_rewrite_mode: next_image_tool_rewrite_mode,
        codex_imagegen_rewrite_mode: next_codex_imagegen_rewrite_mode,
        available_models: next_available_models,
        available_models_mode: next_available_models_mode,
        forward_proxy_key: next_forward_proxy_key,
        forward_proxy_keys_json: next_forward_proxy_keys_json,
    })
}

async fn ensure_binding_write_target(
    pool: &Pool<Sqlite>,
    plan: &PromptCacheBindingWritePlan,
) -> Result<(), ApiError> {
    match plan.binding_kind.as_str() {
        "group" => {
            let group_name = plan.group_name.as_deref().ok_or_else(|| {
                ApiError::bad_request(anyhow!("groupName is required for group binding"))
            })?;
            ensure_group_binding_target(pool, group_name).await?;
        }
        "upstreamAccount" => {
            let account_id = plan.upstream_account_id.ok_or_else(|| {
                ApiError::bad_request(anyhow!(
                    "upstreamAccountId is required for upstream account binding"
                ))
            })?;
            ensure_upstream_account_binding_target(pool, account_id).await?;
        }
        "none" => {}
        _ => {
            return Err(ApiError::bad_request(anyhow!(
                "bindingKind must be one of: none, group, upstreamAccount"
            )));
        }
    }
    Ok(())
}

async fn persist_binding_write_plan(
    state: &AppState,
    prompt_cache_key: &str,
    plan: &PromptCacheBindingWritePlan,
) -> Result<(), ApiError> {
    ensure_binding_write_target(&state.pool, plan).await?;
    let mut connection = state.pool.acquire().await?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(connection.as_mut())
        .await?;
    let result: Result<(), ApiError> = async {
        let all_clear = plan.responses_first_byte_timeout_secs.is_none()
            && plan.compact_first_byte_timeout_secs.is_none()
            && plan.image_first_byte_timeout_secs.is_none()
            && plan.responses_stream_timeout_secs.is_none()
            && plan.compact_stream_timeout_secs.is_none()
            && plan.allow_switch_upstream.is_none()
            && plan.fast_mode_rewrite_mode.is_none()
            && plan.image_tool_rewrite_mode.is_none()
            && plan.codex_imagegen_rewrite_mode.is_none()
            && plan.available_models.is_none()
            && plan.available_models_mode.is_none()
            && plan.forward_proxy_key.is_none()
            && plan.forward_proxy_keys_json.is_none();
        if plan.binding_kind == "none" && all_clear {
            sqlx::query("DELETE FROM prompt_cache_conversation_bindings WHERE prompt_cache_key = ?1")
                .bind(prompt_cache_key)
                .execute(connection.as_mut())
                .await?;
        } else {
            sqlx::query(
                "INSERT INTO prompt_cache_conversation_bindings (prompt_cache_key, binding_kind, group_name, upstream_account_id, responses_first_byte_timeout_secs, compact_first_byte_timeout_secs, image_first_byte_timeout_secs, responses_stream_timeout_secs, compact_stream_timeout_secs, allow_switch_upstream, fast_mode_rewrite_mode, image_tool_rewrite_mode, codex_imagegen_rewrite_mode, available_models_json, available_models_mode, forward_proxy_key, forward_proxy_keys_json, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, datetime('now'), datetime('now')) ON CONFLICT(prompt_cache_key) DO UPDATE SET binding_kind = excluded.binding_kind, group_name = excluded.group_name, upstream_account_id = excluded.upstream_account_id, responses_first_byte_timeout_secs = excluded.responses_first_byte_timeout_secs, compact_first_byte_timeout_secs = excluded.compact_first_byte_timeout_secs, image_first_byte_timeout_secs = excluded.image_first_byte_timeout_secs, responses_stream_timeout_secs = excluded.responses_stream_timeout_secs, compact_stream_timeout_secs = excluded.compact_stream_timeout_secs, allow_switch_upstream = excluded.allow_switch_upstream, fast_mode_rewrite_mode = excluded.fast_mode_rewrite_mode, image_tool_rewrite_mode = excluded.image_tool_rewrite_mode, codex_imagegen_rewrite_mode = excluded.codex_imagegen_rewrite_mode, available_models_json = excluded.available_models_json, available_models_mode = excluded.available_models_mode, forward_proxy_key = excluded.forward_proxy_key, forward_proxy_keys_json = excluded.forward_proxy_keys_json, updated_at = excluded.updated_at",
            )
            .bind(prompt_cache_key)
            .bind(match plan.binding_kind.as_str() {
                "none" => PROMPT_CACHE_BINDING_KIND_NONE,
                "group" => PROMPT_CACHE_BINDING_KIND_GROUP,
                _ => PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT,
            })
            .bind(&plan.group_name)
            .bind(plan.upstream_account_id)
            .bind(plan.responses_first_byte_timeout_secs)
            .bind(plan.compact_first_byte_timeout_secs)
            .bind(plan.image_first_byte_timeout_secs)
            .bind(plan.responses_stream_timeout_secs)
            .bind(plan.compact_stream_timeout_secs)
            .bind(plan.allow_switch_upstream)
            .bind(&plan.fast_mode_rewrite_mode)
            .bind(&plan.image_tool_rewrite_mode)
            .bind(&plan.codex_imagegen_rewrite_mode)
            .bind(&plan.available_models)
            .bind(&plan.available_models_mode)
            .bind(&plan.forward_proxy_key)
            .bind(&plan.forward_proxy_keys_json)
            .execute(connection.as_mut())
            .await?;
        }
        if plan.binding_kind == "upstreamAccount" {
            overwrite_sticky_routes_for_manual_binding_executor(
                connection.as_mut(),
                prompt_cache_key,
                plan.upstream_account_id.expect("validated upstream account id"),
                &format_utc_iso(Utc::now()),
            )
            .await?;
        }
        Ok(())
    }
    .await;
    match result {
        Ok(()) => {
            sqlx::query("COMMIT").execute(connection.as_mut()).await?;
            Ok(())
        }
        Err(error) => {
            let _ = sqlx::query("ROLLBACK").execute(connection.as_mut()).await;
            Err(error)
        }
    }
}

struct PromptCacheBindingEventContext<'a> {
    state: &'a AppState,
    prompt_cache_key: &'a str,
    origin: &'a str,
}

async fn append_binding_change_events(
    context: PromptCacheBindingEventContext<'_>,
    existing_row: Option<&PromptCacheConversationBindingRow>,
    sticky_before: Option<PromptCacheConversationOperationStickySnapshot>,
    next_row: Option<&PromptCacheConversationBindingRow>,
    sticky_after: Option<PromptCacheConversationOperationStickySnapshot>,
    sticky_transitions: &[PromptCacheConversationOperationStickyTransition],
) -> Result<()> {
    let binding_before =
        prompt_cache_conversation_operation_binding_snapshot_from_row(existing_row);
    let binding_after = prompt_cache_conversation_operation_binding_snapshot_from_row(next_row);
    let occurred_at = format_utc_iso(Utc::now());
    if binding_before != binding_after {
        let action = if binding_after.binding_kind == "none" {
            "bindingCleared"
        } else {
            "manualBindingUpdated"
        };
        let input = AppendPromptCacheConversationOperationEventInput {
            prompt_cache_key: context.prompt_cache_key.to_string(),
            action: action.to_string(),
            origin: context.origin.to_string(),
            info_types: vec![PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_ROUTING.to_string()],
            occurred_at: occurred_at.clone(),
            headline: prompt_cache_conversation_operation_headline(action),
            changed_fields: vec!["bindingKind".to_string()],
            binding_before: Some(binding_before.clone()),
            binding_after: Some(binding_after.clone()),
            sticky_before: sticky_before.clone(),
            sticky_after: sticky_after.clone(),
            invoke_id: None,
        };
        if sticky_transitions.is_empty() {
            append_prompt_cache_conversation_operation_event(&context.state.pool, input).await?;
        } else {
            append_prompt_cache_conversation_operation_event_with_sticky_transitions_executor(
                &context.state.pool,
                input,
                sticky_transitions,
            )
            .await?;
        }
    }
    let policy_changed_fields =
        prompt_cache_conversation_policy_changed_fields(existing_row, next_row);
    if !policy_changed_fields.is_empty() {
        append_prompt_cache_conversation_operation_event(
            &context.state.pool,
            AppendPromptCacheConversationOperationEventInput {
                prompt_cache_key: context.prompt_cache_key.to_string(),
                action: "conversationPolicyUpdated".to_string(),
                origin: context.origin.to_string(),
                info_types: prompt_cache_conversation_policy_info_types(&policy_changed_fields),
                occurred_at: occurred_at.clone(),
                headline: prompt_cache_conversation_operation_headline("conversationPolicyUpdated"),
                changed_fields: policy_changed_fields,
                binding_before: Some(binding_before.clone()),
                binding_after: Some(binding_after.clone()),
                sticky_before: sticky_before.clone(),
                sticky_after: sticky_after.clone(),
                invoke_id: None,
            },
        )
        .await?;
    }
    if sticky_before != sticky_after && binding_before == binding_after {
        let (action, sticky_after_event) = match sticky_after {
            Some(snapshot) => ("stickyTargetChanged", Some(snapshot)),
            None => ("stickyTargetCleared", None),
        };
        append_prompt_cache_conversation_operation_event(
            &context.state.pool,
            AppendPromptCacheConversationOperationEventInput {
                prompt_cache_key: context.prompt_cache_key.to_string(),
                action: action.to_string(),
                origin: context.origin.to_string(),
                info_types: vec![PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_ROUTING.to_string()],
                occurred_at,
                headline: prompt_cache_conversation_operation_headline(action),
                changed_fields: vec!["stickyTarget".to_string()],
                binding_before: None,
                binding_after: None,
                sticky_before,
                sticky_after: sticky_after_event,
                invoke_id: None,
            },
        )
        .await?;
    }
    Ok(())
}

async fn save_prompt_cache_conversation_binding_for_key(
    state: &AppState,
    prompt_cache_key: &str,
    payload: UpdatePromptCacheConversationBindingRequest,
    origin: &str,
) -> Result<PromptCacheConversationBindingResponse, ApiError> {
    let _write_guard = PROMPT_CACHE_BINDING_WRITE_LOCK.lock().await;
    let update = normalize_binding_update(state, payload).await?;
    let existing_row =
        load_prompt_cache_conversation_binding_row(&state.pool, prompt_cache_key).await?;
    let sticky_before =
        load_prompt_cache_conversation_sticky_snapshot(&state.pool, prompt_cache_key).await?;
    let sticky_routes_before =
        load_prompt_cache_conversation_sticky_routes(&state.pool, prompt_cache_key).await?;
    let plan = build_binding_write_plan(existing_row.as_ref(), update)?;

    persist_binding_write_plan(state, prompt_cache_key, &plan).await?;

    let next_row =
        load_prompt_cache_conversation_binding_row(&state.pool, prompt_cache_key).await?;
    let sticky_after =
        load_prompt_cache_conversation_sticky_snapshot(&state.pool, prompt_cache_key).await?;
    let sticky_routes_after =
        load_prompt_cache_conversation_sticky_routes(&state.pool, prompt_cache_key).await?;
    let sticky_transitions =
        prompt_cache_conversation_sticky_transitions(&sticky_routes_before, &sticky_routes_after);
    append_binding_change_events(
        PromptCacheBindingEventContext {
            state,
            prompt_cache_key,
            origin,
        },
        existing_row.as_ref(),
        sticky_before,
        next_row.as_ref(),
        sticky_after,
        &sticky_transitions,
    )
    .await?;

    broadcast_prompt_cache_conversation_changed(state, prompt_cache_key).await;
    load_prompt_cache_conversation_binding_response_for_key(state, prompt_cache_key.to_string())
        .await
}

pub(crate) async fn get_prompt_cache_conversation_binding(
    State(state): State<Arc<AppState>>,
    AxumPath(encoded_prompt_cache_key): AxumPath<String>,
) -> Result<Json<PromptCacheConversationBindingResponse>, ApiError> {
    let prompt_cache_key = normalize_prompt_cache_conversation_key(&encoded_prompt_cache_key)?;
    Ok(Json(
        load_prompt_cache_conversation_binding_response_for_key(state.as_ref(), prompt_cache_key)
            .await?,
    ))
}

pub(crate) async fn patch_prompt_cache_conversation_binding(
    State(state): State<Arc<AppState>>,
    AxumPath(encoded_prompt_cache_key): AxumPath<String>,
    Json(payload): Json<UpdatePromptCacheConversationBindingRequest>,
) -> Result<Json<PromptCacheConversationBindingResponse>, ApiError> {
    let prompt_cache_key = normalize_prompt_cache_conversation_key(&encoded_prompt_cache_key)?;
    Ok(Json(
        save_prompt_cache_conversation_binding_for_key(
            state.as_ref(),
            &prompt_cache_key,
            payload,
            PROMPT_CACHE_CONVERSATION_OPERATION_ORIGIN_DETAIL_DRAWER,
        )
        .await?,
    ))
}

pub(crate) async fn post_prompt_cache_conversation_affinity_reset(
    State(state): State<Arc<AppState>>,
    AxumPath(encoded_prompt_cache_key): AxumPath<String>,
) -> Result<Json<PromptCacheConversationBindingResponse>, ApiError> {
    let prompt_cache_key = normalize_prompt_cache_conversation_key(&encoded_prompt_cache_key)?;
    Ok(Json(
        clear_prompt_cache_conversation_affinity(
            state.as_ref(),
            &prompt_cache_key,
            PROMPT_CACHE_CONVERSATION_OPERATION_ORIGIN_DETAIL_DRAWER,
        )
        .await?,
    ))
}

async fn count_prompt_cache_conversation_operation_events(
    state: &AppState,
    prompt_cache_key: &str,
    info_type: Option<&str>,
    routing_scope: Option<&str>,
    routing_model: Option<&str>,
) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM prompt_cache_conversation_operation_events
        WHERE prompt_cache_key = ?1
          AND (?2 IS NULL OR EXISTS (
              SELECT 1 FROM json_each(info_types_json) WHERE json_each.value = ?2
          ))
          AND (?3 IS NULL OR json_extract(routing_scope_json, '$.kind') = ?3)
          AND (?4 IS NULL OR json_extract(routing_scope_json, '$.modelKey') = ?4)
        "#,
    )
    .bind(prompt_cache_key)
    .bind(info_type)
    .bind(routing_scope)
    .bind(routing_model)
    .fetch_one(&state.pool)
    .await?)
}

async fn load_prompt_cache_conversation_operation_events(
    state: &AppState,
    prompt_cache_key: &str,
    info_type: Option<&str>,
    routing_scope: Option<&str>,
    routing_model: Option<&str>,
    page_size: usize,
    offset: usize,
) -> Result<Vec<PromptCacheConversationOperationEventRow>> {
    Ok(
        sqlx::query_as::<_, PromptCacheConversationOperationEventRow>(
            r#"
        SELECT id, prompt_cache_key, action, origin, info_types_json, occurred_at, headline,
               changed_fields_json, binding_before_json, binding_after_json, sticky_before_json,
               sticky_after_json, invoke_id, routing_context_json, routing_scope_json,
               sticky_transitions_json
        FROM prompt_cache_conversation_operation_events
        WHERE prompt_cache_key = ?1
          AND (?2 IS NULL OR EXISTS (
              SELECT 1 FROM json_each(info_types_json) WHERE json_each.value = ?2
          ))
          AND (?3 IS NULL OR json_extract(routing_scope_json, '$.kind') = ?3)
          AND (?4 IS NULL OR json_extract(routing_scope_json, '$.modelKey') = ?4)
        ORDER BY occurred_at DESC, id DESC
        LIMIT ?5 OFFSET ?6
        "#,
        )
        .bind(prompt_cache_key)
        .bind(info_type)
        .bind(routing_scope)
        .bind(routing_model)
        .bind(page_size as i64)
        .bind(offset as i64)
        .fetch_all(&state.pool)
        .await?,
    )
}

async fn load_prompt_cache_conversation_operation_model_facets(
    state: &AppState,
    prompt_cache_key: &str,
) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar::<_, String>(
        r#"
        SELECT DISTINCT json_extract(routing_scope_json, '$.modelKey')
        FROM prompt_cache_conversation_operation_events
        WHERE prompt_cache_key = ?1
          AND json_extract(routing_scope_json, '$.kind') = 'model'
          AND json_extract(routing_scope_json, '$.modelKey') IS NOT NULL
        ORDER BY 1 ASC
        "#,
    )
    .bind(prompt_cache_key)
    .fetch_all(&state.pool)
    .await?)
}

pub(crate) async fn list_prompt_cache_conversation_operation_events(
    State(state): State<Arc<AppState>>,
    AxumPath(encoded_prompt_cache_key): AxumPath<String>,
    axum::extract::Query(query): axum::extract::Query<
        ListPromptCacheConversationOperationEventsQuery,
    >,
) -> Result<Json<PromptCacheConversationOperationEventListResponse>, ApiError> {
    let prompt_cache_key = normalize_prompt_cache_conversation_key(&encoded_prompt_cache_key)?;
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query
        .page_size
        .unwrap_or(PROMPT_CACHE_CONVERSATION_OPERATION_EVENTS_DEFAULT_PAGE_SIZE)
        .clamp(1, PROMPT_CACHE_CONVERSATION_OPERATION_EVENTS_MAX_PAGE_SIZE);
    let info_type =
        normalize_prompt_cache_conversation_operation_info_type(query.info_type.as_deref())?;
    let routing_scope = normalize_prompt_cache_conversation_operation_routing_scope(
        query.routing_scope.as_deref(),
    )?;
    let routing_model = normalize_prompt_cache_conversation_operation_routing_model(
        query.routing_model.as_deref(),
    )?;
    let offset = (page - 1) * page_size;

    let total = count_prompt_cache_conversation_operation_events(
        &state,
        &prompt_cache_key,
        info_type.as_deref(),
        routing_scope.as_deref(),
        routing_model.as_deref(),
    )
    .await?;
    let rows = load_prompt_cache_conversation_operation_events(
        &state,
        &prompt_cache_key,
        info_type.as_deref(),
        routing_scope.as_deref(),
        routing_model.as_deref(),
        page_size,
        offset,
    )
    .await?;
    let routing_model_facets =
        load_prompt_cache_conversation_operation_model_facets(&state, &prompt_cache_key).await?;

    Ok(Json(PromptCacheConversationOperationEventListResponse {
        items: rows
            .into_iter()
            .map(prompt_cache_conversation_operation_event_response_from_row)
            .collect(),
        total,
        page,
        page_size,
        routing_model_facets,
    }))
}

fn validate_bulk_prompt_cache_binding_action(
    action: &BulkPromptCacheConversationBindingsAction,
) -> Result<(), ApiError> {
    match action {
        BulkPromptCacheConversationBindingsAction::Bind {
            binding_kind,
            group_name,
            upstream_account_id,
        } => {
            let binding_kind = binding_kind.trim();
            let group_name = group_name
                .as_ref()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty());
            if group_name.is_some() && upstream_account_id.is_some() {
                return Err(ApiError::bad_request(anyhow!(
                    "groupName and upstreamAccountId are mutually exclusive"
                )));
            }
            match binding_kind {
                "group" if group_name.is_none() => Err(ApiError::bad_request(anyhow!(
                    "groupName is required for group binding"
                ))),
                "upstreamAccount" if upstream_account_id.is_none() => Err(ApiError::bad_request(
                    anyhow!("upstreamAccountId is required for upstream account binding"),
                )),
                "none" if group_name.is_some() || upstream_account_id.is_some() => {
                    Err(ApiError::bad_request(anyhow!(
                        "groupName and upstreamAccountId must be omitted when clearing binding"
                    )))
                }
                "none" | "group" | "upstreamAccount" => Ok(()),
                _ => Err(ApiError::bad_request(anyhow!(
                    "bindingKind must be one of: none, group, upstreamAccount"
                ))),
            }
        }
        BulkPromptCacheConversationBindingsAction::SetFastModeRewriteMode {
            fast_mode_rewrite_mode,
        } => {
            let normalized_mode = normalize_fast_mode_rewrite_mode(PatchField::Value(
                fast_mode_rewrite_mode.clone(),
            ))?;
            if matches!(normalized_mode, PatchField::Value(_)) {
                Ok(())
            } else {
                Err(ApiError::bad_request(anyhow!(
                    "fastModeRewriteMode is required"
                )))
            }
        }
        BulkPromptCacheConversationBindingsAction::ClearAndResetAffinity => Ok(()),
    }
}

async fn apply_bulk_prompt_cache_binding_action(
    state: &AppState,
    prompt_cache_key: &str,
    action: &BulkPromptCacheConversationBindingsAction,
) -> Result<PromptCacheConversationBindingResponse, ApiError> {
    match action {
        BulkPromptCacheConversationBindingsAction::Bind {
            binding_kind,
            group_name,
            upstream_account_id,
        } => {
            if binding_kind.trim() == "group" {
                ensure_group_binding_target(&state.pool, group_name.as_deref().unwrap()).await?;
            } else if binding_kind.trim() == "upstreamAccount" {
                ensure_upstream_account_binding_target(&state.pool, upstream_account_id.unwrap())
                    .await?;
            }
            save_prompt_cache_conversation_binding_for_key(
                state,
                prompt_cache_key,
                UpdatePromptCacheConversationBindingRequest {
                    binding_kind: binding_kind.clone(),
                    group_name: group_name.clone(),
                    upstream_account_id: *upstream_account_id,
                    timeouts: None,
                    allow_switch_upstream: PatchField::Missing,
                    fast_mode_rewrite_mode: PatchField::Missing,
                    image_tool_rewrite_mode: PatchField::Missing,
                    codex_imagegen_rewrite_mode: PatchField::Missing,
                    available_models: PatchField::Missing,
                    available_models_mode: PatchField::Missing,
                    forward_proxy_key: PatchField::Missing,
                    forward_proxy_keys: PatchField::Missing,
                },
                PROMPT_CACHE_CONVERSATION_OPERATION_ORIGIN_DASHBOARD_BULK,
            )
            .await
        }
        BulkPromptCacheConversationBindingsAction::ClearAndResetAffinity => {
            clear_prompt_cache_conversation_affinity(
                state,
                prompt_cache_key,
                PROMPT_CACHE_CONVERSATION_OPERATION_ORIGIN_DASHBOARD_BULK,
            )
            .await
        }
        BulkPromptCacheConversationBindingsAction::SetFastModeRewriteMode {
            fast_mode_rewrite_mode,
        } => {
            let existing_row =
                load_prompt_cache_conversation_binding_row(&state.pool, prompt_cache_key).await?;
            let mut request = existing_binding_request(existing_row.as_ref());
            request.fast_mode_rewrite_mode = PatchField::Value(fast_mode_rewrite_mode.clone());
            save_prompt_cache_conversation_binding_for_key(
                state,
                prompt_cache_key,
                request,
                PROMPT_CACHE_CONVERSATION_OPERATION_ORIGIN_DASHBOARD_BULK,
            )
            .await
        }
    }
}

async fn validate_bulk_prompt_cache_binding_target(
    state: &AppState,
    action: &BulkPromptCacheConversationBindingsAction,
) -> Result<(), ApiError> {
    let BulkPromptCacheConversationBindingsAction::Bind {
        binding_kind,
        group_name,
        upstream_account_id,
    } = action
    else {
        return Ok(());
    };
    match binding_kind.trim() {
        "group" => {
            let group_name = group_name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .expect("validated group binding requires a group name");
            ensure_group_binding_target(&state.pool, group_name).await
        }
        "upstreamAccount" => {
            let upstream_account_id = upstream_account_id
                .expect("validated upstream account binding requires an account id");
            ensure_upstream_account_binding_target(&state.pool, upstream_account_id)
                .await
                .map(|_| ())
        }
        _ => Ok(()),
    }
}

pub(crate) async fn post_bulk_prompt_cache_conversation_bindings(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<BulkPromptCacheConversationBindingsRequest>,
) -> Result<Json<BulkPromptCacheConversationBindingsResponse>, ApiError> {
    let prompt_cache_keys = normalized_prompt_cache_conversation_keys(payload.prompt_cache_keys)?;
    validate_bulk_prompt_cache_binding_action(&payload.action)?;
    validate_bulk_prompt_cache_binding_target(state.as_ref(), &payload.action).await?;

    let mut items = Vec::with_capacity(prompt_cache_keys.len());
    let mut total_succeeded = 0usize;
    for prompt_cache_key in prompt_cache_keys {
        let result = apply_bulk_prompt_cache_binding_action(
            state.as_ref(),
            &prompt_cache_key,
            &payload.action,
        )
        .await;
        match result {
            Ok(binding) => {
                total_succeeded += 1;
                items.push(BulkPromptCacheConversationBindingItemResponse {
                    prompt_cache_key,
                    ok: true,
                    error: None,
                    binding: Some(binding),
                });
            }
            Err(err) => {
                items.push(BulkPromptCacheConversationBindingItemResponse {
                    prompt_cache_key,
                    ok: false,
                    error: Some(match err {
                        ApiError::BadRequest(err)
                        | ApiError::Unavailable(err)
                        | ApiError::Internal(err) => err.to_string(),
                    }),
                    binding: None,
                });
            }
        }
    }
    let total_requested = items.len();
    let total_failed = total_requested.saturating_sub(total_succeeded);
    let action = match payload.action {
        BulkPromptCacheConversationBindingsAction::Bind { .. } => "bind",
        BulkPromptCacheConversationBindingsAction::ClearAndResetAffinity => "clearAndResetAffinity",
        BulkPromptCacheConversationBindingsAction::SetFastModeRewriteMode { .. } => {
            "setFastModeRewriteMode"
        }
    }
    .to_string();
    Ok(Json(BulkPromptCacheConversationBindingsResponse {
        action,
        total_requested,
        total_succeeded,
        total_failed,
        items,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_mode_changes_are_request_rewrite_events() {
        let info_types =
            prompt_cache_conversation_policy_info_types(&["availableModelsMode".to_string()]);

        assert_eq!(
            info_types,
            vec![PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_REQUEST_REWRITE.to_string()]
        );
    }

    #[test]
    fn available_models_parser_fails_closed_on_blank_entries() {
        assert_eq!(
            parse_available_models_json_with_invalid(r#"[" "]"#),
            (Some(Vec::new()), true)
        );
        assert_eq!(
            parse_available_models_json_with_invalid("[]"),
            (Some(Vec::new()), false)
        );
    }
}
