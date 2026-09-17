pub(crate) async fn binding_response_for_none(
    state: &AppState,
    config: &AppConfig,
    prompt_cache_key: String,
    owner: Option<&PromptCacheEncryptedSessionOwnerRow>,
) -> Result<PromptCacheConversationBindingResponse> {
    let sticky_account_id = load_sticky_account_id(&state.pool, &prompt_cache_key).await?;
    let effective_account_id = owner
        .map(|owner| owner.owner_upstream_account_id)
        .or(sticky_account_id);
    let effective_policy = if let Some(account_id) = effective_account_id {
        load_effective_routing_rule_for_account(&state.pool, account_id).await?
    } else {
        load_effective_routing_rule_for_group(&state.pool, None).await?
    };
    let (timeouts, timeout_field_sources) = if let Some(owner) = owner {
        let (timeouts, sources, _) = load_effective_request_path_timeouts_for_account(
            &state.pool,
            config,
            owner.owner_upstream_account_id,
            Some(prompt_cache_key.as_str()),
        )
        .await?;
        (timeouts, sources)
    } else {
        let (timeouts, sources, _) =
            load_effective_request_path_timeouts_for_group_and_conversation(
                &state.pool,
                config,
                None,
                Some(prompt_cache_key.as_str()),
            )
            .await?;
        (timeouts, sources)
    };
    let forward_proxy_key =
        resolve_effective_forward_proxy_key_for_account(state, effective_account_id).await;
    let forward_proxy_keys = forward_proxy_key.iter().cloned().collect::<Vec<_>>();
    let sticky_routes =
        load_prompt_cache_conversation_sticky_routes(&state.pool, &prompt_cache_key).await?;

    Ok(PromptCacheConversationBindingResponse {
        prompt_cache_key,
        binding_kind: "none".to_string(),
        group_name: None,
        upstream_account_id: None,
        upstream_account_name: None,
        has_encrypted_session_owner: false,
        encrypted_owner_account_id: None,
        encrypted_owner_account_name: None,
        encrypted_owner_group_name: None,
        sticky_routes,
        timeouts,
        timeout_field_sources,
        allow_switch_upstream: Some(effective_policy.allow_cut_out()),
        fast_mode_rewrite_mode: Some(effective_policy.fast_mode_rewrite_mode),
        image_tool_rewrite_mode: Some(effective_policy.image_tool_rewrite_mode),
        codex_imagegen_rewrite_mode: Some(effective_policy.codex_imagegen_rewrite_mode),
        available_models: effective_policy
            .available_models()
            .map(|models| models.to_vec()),
        available_models_mode: Some(effective_policy.available_models_mode),
        forward_proxy_key,
        forward_proxy_keys,
        policy_field_sources: PromptCacheConversationPolicyFieldSources::inherited(Some(
            &effective_policy,
        )),
        updated_at: None,
    })
}

async fn resolve_binding_effective_policy(
    state: &AppState,
    row: &PromptCacheConversationBindingRow,
    owner: Option<&PromptCacheEncryptedSessionOwnerRow>,
    sticky_account_id: Option<i64>,
) -> Result<Option<EffectiveRoutingRule>> {
    if row.binding_kind == PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT {
        return match row.upstream_account_id {
            Some(account_id) => Ok(Some(
                load_effective_routing_rule_for_account(&state.pool, account_id).await?,
            )),
            None => Ok(None),
        };
    }
    if row.binding_kind == PROMPT_CACHE_BINDING_KIND_GROUP {
        return Ok(Some(
            load_effective_routing_rule_for_group(&state.pool, row.group_name.as_deref()).await?,
        ));
    }
    if row.binding_kind != PROMPT_CACHE_BINDING_KIND_NONE {
        return Ok(None);
    }
    if let Some(owner) = owner {
        return Ok(Some(
            load_effective_routing_rule_for_account(&state.pool, owner.owner_upstream_account_id)
                .await?,
        ));
    }
    match sticky_account_id {
        Some(account_id) => Ok(Some(
            load_effective_routing_rule_for_account(&state.pool, account_id).await?,
        )),
        None => Ok(Some(
            load_effective_routing_rule_for_group(&state.pool, None).await?,
        )),
    }
}

async fn resolve_binding_timeouts(
    state: &AppState,
    config: &AppConfig,
    row: &PromptCacheConversationBindingRow,
    owner: Option<&PromptCacheEncryptedSessionOwnerRow>,
) -> Result<(RoutingTimeoutSettings, RoutingTimeoutFieldSources)> {
    let prompt_cache_key = Some(row.prompt_cache_key.as_str());
    let (timeouts, sources, _) = if row.binding_kind == PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT {
        match row.upstream_account_id {
            Some(account_id) => {
                load_effective_request_path_timeouts_for_account(
                    &state.pool,
                    config,
                    account_id,
                    prompt_cache_key,
                )
                .await?
            }
            None => {
                load_effective_request_path_timeouts_for_group_and_conversation(
                    &state.pool,
                    config,
                    None,
                    prompt_cache_key,
                )
                .await?
            }
        }
    } else if row.binding_kind == PROMPT_CACHE_BINDING_KIND_GROUP {
        load_effective_request_path_timeouts_for_group_and_conversation(
            &state.pool,
            config,
            row.group_name.as_deref(),
            prompt_cache_key,
        )
        .await?
    } else if let Some(owner) = owner {
        load_effective_request_path_timeouts_for_account(
            &state.pool,
            config,
            owner.owner_upstream_account_id,
            prompt_cache_key,
        )
        .await?
    } else {
        load_effective_request_path_timeouts_for_group_and_conversation(
            &state.pool,
            config,
            None,
            prompt_cache_key,
        )
        .await?
    };
    Ok((timeouts, sources))
}

struct PromptCacheBindingResponseData {
    effective_policy: Option<EffectiveRoutingRule>,
    legacy_available_models: Option<Vec<String>>,
    available_models_invalid: bool,
    effective_available_models: Option<Vec<String>>,
    timeouts: RoutingTimeoutSettings,
    timeout_field_sources: RoutingTimeoutFieldSources,
    forward_proxy_key: Option<String>,
    forward_proxy_keys: Vec<String>,
    sticky_routes: Vec<PromptCacheConversationStickyRouteResponse>,
}

async fn load_binding_response_data(
    state: &AppState,
    config: &AppConfig,
    row: &PromptCacheConversationBindingRow,
    owner: Option<&PromptCacheEncryptedSessionOwnerRow>,
    sticky_account_id: Option<i64>,
) -> Result<PromptCacheBindingResponseData> {
    let effective_policy =
        resolve_binding_effective_policy(state, row, owner, sticky_account_id).await?;
    let (legacy_available_models, available_models_invalid) = row
        .available_models_json
        .as_deref()
        .map(parse_available_models_json_with_invalid)
        .unwrap_or((None, false));
    let effective_available_models = effective_policy
        .as_ref()
        .and_then(|rule| rule.available_models().map(|models| models.to_vec()));
    let (timeouts, timeout_field_sources) =
        resolve_binding_timeouts(state, config, row, owner).await?;
    let row_forward_proxy_keys =
        parse_forward_proxy_keys_json(row.forward_proxy_keys_json.as_deref());
    let forward_proxy_key = row
        .forward_proxy_key
        .clone()
        .or_else(|| row_forward_proxy_keys.first().cloned())
        .or(
            resolve_effective_forward_proxy_key_for_row(state, row, owner, sticky_account_id).await,
        );
    let forward_proxy_keys = if row_forward_proxy_keys.is_empty() {
        forward_proxy_key.iter().cloned().collect()
    } else {
        row_forward_proxy_keys
    };
    let sticky_routes =
        load_prompt_cache_conversation_sticky_routes(&state.pool, &row.prompt_cache_key).await?;
    Ok(PromptCacheBindingResponseData {
        effective_policy,
        legacy_available_models,
        available_models_invalid,
        effective_available_models,
        timeouts,
        timeout_field_sources,
        forward_proxy_key,
        forward_proxy_keys,
        sticky_routes,
    })
}

fn build_binding_response(
    row: PromptCacheConversationBindingRow,
    owner: Option<&PromptCacheEncryptedSessionOwnerRow>,
    data: PromptCacheBindingResponseData,
) -> PromptCacheConversationBindingResponse {
    let PromptCacheBindingResponseData {
        effective_policy,
        legacy_available_models,
        available_models_invalid,
        effective_available_models,
        timeouts,
        timeout_field_sources,
        forward_proxy_key,
        forward_proxy_keys,
        sticky_routes,
    } = data;
    let policy_field_sources =
        PromptCacheConversationPolicyFieldSources::from_row(&row, effective_policy.as_ref());
    PromptCacheConversationBindingResponse {
        prompt_cache_key: row.prompt_cache_key,
        binding_kind: match row.binding_kind.as_str() {
            PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT => "upstreamAccount".to_string(),
            PROMPT_CACHE_BINDING_KIND_NONE => "none".to_string(),
            _ => "group".to_string(),
        },
        group_name: row.group_name,
        upstream_account_id: row.upstream_account_id,
        upstream_account_name: row.upstream_account_name,
        has_encrypted_session_owner: owner.is_some(),
        encrypted_owner_account_id: owner.map(|value| value.owner_upstream_account_id),
        encrypted_owner_account_name: owner
            .and_then(|value| value.owner_upstream_account_name.clone()),
        encrypted_owner_group_name: owner.and_then(|value| value.owner_group_name.clone()),
        sticky_routes,
        timeouts,
        timeout_field_sources,
        allow_switch_upstream: row
            .allow_switch_upstream
            .map(|value| value != 0)
            .or_else(|| effective_policy.as_ref().map(|rule| rule.allow_cut_out())),
        fast_mode_rewrite_mode: row
            .fast_mode_rewrite_mode
            .as_deref()
            .map(parse_fast_mode_rewrite_mode_lossy)
            .or_else(|| {
                effective_policy
                    .as_ref()
                    .map(|rule| rule.fast_mode_rewrite_mode)
            }),
        image_tool_rewrite_mode: row
            .image_tool_rewrite_mode
            .as_deref()
            .map(ImageToolRewriteMode::from_str)
            .or_else(|| {
                effective_policy
                    .as_ref()
                    .map(|rule| rule.image_tool_rewrite_mode)
            }),
        codex_imagegen_rewrite_mode: row
            .codex_imagegen_rewrite_mode
            .as_deref()
            .map(CodexImagegenRewriteMode::from_str)
            .or_else(|| {
                effective_policy
                    .as_ref()
                    .map(|rule| rule.codex_imagegen_rewrite_mode)
            }),
        available_models: if available_models_invalid {
            Some(Vec::new())
        } else {
            legacy_available_models
                .clone()
                .or(effective_available_models)
        },
        available_models_mode: if available_models_invalid {
            Some(AvailableModelsMode::Allowlist)
        } else {
            row.available_models_mode
                .as_deref()
                .map(|value| AvailableModelsMode::from_str(Some(value)))
                .or_else(|| {
                    legacy_available_models
                        .as_ref()
                        .map(|_| AvailableModelsMode::Allowlist)
                })
                .or_else(|| {
                    effective_policy
                        .as_ref()
                        .map(|rule| rule.available_models_mode)
                })
        },
        forward_proxy_key,
        forward_proxy_keys,
        policy_field_sources,
        updated_at: Some(row.updated_at),
    }
}

pub(crate) async fn binding_response_from_row(
    state: &AppState,
    config: &AppConfig,
    row: PromptCacheConversationBindingRow,
    owner: Option<&PromptCacheEncryptedSessionOwnerRow>,
) -> Result<PromptCacheConversationBindingResponse> {
    let sticky_account_id = if row.binding_kind == PROMPT_CACHE_BINDING_KIND_NONE {
        load_sticky_account_id(&state.pool, &row.prompt_cache_key).await?
    } else {
        None
    };
    let data = load_binding_response_data(state, config, &row, owner, sticky_account_id).await?;
    Ok(build_binding_response(row, owner, data))
}

impl PromptCacheConversationPolicyFieldSources {
    fn inherited(effective_policy: Option<&EffectiveRoutingRule>) -> Self {
        Self {
            allow_switch_upstream: effective_policy
                .map(|rule| rule.allow_cut_out_source())
                .unwrap_or("root")
                .to_string(),
            fast_mode_rewrite_mode: effective_policy
                .map(|rule| rule.fast_mode_rewrite_mode_source())
                .unwrap_or("root")
                .to_string(),
            image_tool_rewrite_mode: effective_policy
                .map(|rule| rule.image_tool_rewrite_mode_source())
                .unwrap_or("root")
                .to_string(),
            codex_imagegen_rewrite_mode: effective_policy
                .map(|rule| rule.codex_imagegen_rewrite_mode_source())
                .unwrap_or("root")
                .to_string(),
            available_models: effective_policy
                .map(|rule| rule.available_models_source())
                .unwrap_or("root")
                .to_string(),
            available_models_mode: effective_policy
                .map(|rule| rule.available_models_mode_source())
                .unwrap_or("root")
                .to_string(),
            forward_proxy_key: "account".to_string(),
        }
    }

    fn from_row(
        row: &PromptCacheConversationBindingRow,
        effective_policy: Option<&EffectiveRoutingRule>,
    ) -> Self {
        Self {
            allow_switch_upstream: source_for_optional_or_effective(
                row.allow_switch_upstream.as_ref(),
                effective_policy.map(|rule| rule.allow_cut_out_source()),
            ),
            fast_mode_rewrite_mode: source_for_optional_or_effective(
                row.fast_mode_rewrite_mode.as_ref(),
                effective_policy.map(|rule| rule.fast_mode_rewrite_mode_source()),
            ),
            image_tool_rewrite_mode: source_for_optional_or_effective(
                row.image_tool_rewrite_mode.as_ref(),
                effective_policy.map(|rule| rule.image_tool_rewrite_mode_source()),
            ),
            codex_imagegen_rewrite_mode: source_for_optional_or_effective(
                row.codex_imagegen_rewrite_mode.as_ref(),
                effective_policy.map(|rule| rule.codex_imagegen_rewrite_mode_source()),
            ),
            available_models: source_for_optional_or_effective(
                row.available_models_json.as_ref(),
                effective_policy.map(|rule| rule.available_models_source()),
            ),
            available_models_mode: if row.available_models_mode.is_some()
                || row.available_models_json.is_some()
            {
                "conversation".to_string()
            } else {
                effective_policy
                    .map(|rule| rule.available_models_mode_source())
                    .unwrap_or("account")
                    .to_string()
            },
            forward_proxy_key: source_for_optional(
                row.forward_proxy_key
                    .as_ref()
                    .or(row.forward_proxy_keys_json.as_ref()),
            ),
        }
    }
}

pub(crate) fn source_for_optional<T>(value: Option<&T>) -> String {
    if value.is_some() {
        "conversation"
    } else {
        "account"
    }
    .to_string()
}

pub(crate) fn source_for_optional_or_effective<T>(
    value: Option<&T>,
    effective_source: Option<&str>,
) -> String {
    if value.is_some() {
        "conversation"
    } else {
        effective_source.unwrap_or("root")
    }
    .to_string()
}

pub(crate) async fn load_sticky_account_id(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
) -> Result<Option<i64>> {
    sqlx::query_scalar::<_, i64>(
        "SELECT account_id FROM pool_sticky_routes WHERE sticky_key = ?1 LIMIT 1",
    )
    .bind(prompt_cache_key)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn load_prompt_cache_conversation_sticky_routes(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
) -> Result<Vec<PromptCacheConversationStickyRouteResponse>> {
    load_prompt_cache_conversation_sticky_routes_executor(pool, prompt_cache_key).await
}

async fn load_prompt_cache_conversation_sticky_routes_executor<'e, E>(
    executor: E,
    prompt_cache_key: &str,
) -> Result<Vec<PromptCacheConversationStickyRouteResponse>>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    sqlx::query_as::<_, PromptCacheConversationStickyRouteResponse>(
        r#"
        SELECT
            route.model_key,
            route.account_id AS upstream_account_id,
            account.display_name AS upstream_account_name,
            route.created_at,
            route.updated_at,
            route.last_seen_at
        FROM (
            SELECT NULL AS model_key, sticky_key, account_id, created_at, updated_at, last_seen_at
            FROM pool_sticky_routes
            WHERE sticky_key = ?1
            UNION ALL
            SELECT model_key, sticky_key, account_id, created_at, updated_at, last_seen_at
            FROM pool_sticky_model_routes
            WHERE sticky_key = ?1
        ) AS route
        LEFT JOIN pool_upstream_accounts AS account
          ON account.id = route.account_id
        ORDER BY route.model_key IS NOT NULL ASC, route.model_key ASC
        "#,
    )
    .bind(prompt_cache_key)
    .fetch_all(executor)
    .await
    .map_err(Into::into)
}

pub(crate) async fn load_prompt_cache_conversation_sticky_snapshot_executor<'e, E>(
    executor: E,
    prompt_cache_key: &str,
) -> Result<Option<PromptCacheConversationOperationStickySnapshot>>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    #[derive(Debug, FromRow)]
    struct StickySnapshotRow {
        account_id: i64,
        account_name: Option<String>,
    }

    let row = sqlx::query_as::<_, StickySnapshotRow>(
        r#"
        SELECT
            sticky.account_id,
            account.display_name AS account_name
        FROM pool_sticky_routes AS sticky
        LEFT JOIN pool_upstream_accounts AS account
          ON account.id = sticky.account_id
        WHERE sticky.sticky_key = ?1
        LIMIT 1
        "#,
    )
    .bind(prompt_cache_key)
    .fetch_optional(executor)
    .await?;
    Ok(
        row.map(|value| PromptCacheConversationOperationStickySnapshot {
            upstream_account_id: value.account_id,
            upstream_account_name: value.account_name,
        }),
    )
}

async fn load_prompt_cache_conversation_sticky_snapshot_for_model_executor<'e, E>(
    executor: E,
    prompt_cache_key: &str,
    model_key: Option<&str>,
) -> Result<Option<PromptCacheConversationOperationStickySnapshot>>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let Some(model_key) = model_key else {
        return load_prompt_cache_conversation_sticky_snapshot_executor(executor, prompt_cache_key)
            .await;
    };

    #[derive(Debug, FromRow)]
    struct StickySnapshotRow {
        account_id: i64,
        account_name: Option<String>,
    }

    sqlx::query_as::<_, StickySnapshotRow>(
        r#"
        SELECT
            sticky.account_id,
            account.display_name AS account_name
        FROM pool_sticky_model_routes AS sticky
        LEFT JOIN pool_upstream_accounts AS account
          ON account.id = sticky.account_id
        WHERE sticky.sticky_key = ?1 AND sticky.model_key = ?2
        LIMIT 1
        "#,
    )
    .bind(prompt_cache_key)
    .bind(model_key)
    .fetch_optional(executor)
    .await
    .map(|row| {
        row.map(|value| PromptCacheConversationOperationStickySnapshot {
            upstream_account_id: value.account_id,
            upstream_account_name: value.account_name,
        })
    })
    .map_err(Into::into)
}

pub(crate) async fn load_prompt_cache_conversation_sticky_snapshot(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
) -> Result<Option<PromptCacheConversationOperationStickySnapshot>> {
    load_prompt_cache_conversation_sticky_snapshot_executor(pool, prompt_cache_key).await
}

fn prompt_cache_conversation_operation_binding_snapshot_from_row(
    row: Option<&PromptCacheConversationBindingRow>,
) -> PromptCacheConversationOperationBindingSnapshot {
    match row {
        Some(value) if value.binding_kind == PROMPT_CACHE_BINDING_KIND_GROUP => {
            PromptCacheConversationOperationBindingSnapshot {
                binding_kind: "group".to_string(),
                group_name: value.group_name.clone(),
                upstream_account_id: None,
                upstream_account_name: None,
            }
        }
        Some(value) if value.binding_kind == PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT => {
            PromptCacheConversationOperationBindingSnapshot {
                binding_kind: "upstreamAccount".to_string(),
                group_name: None,
                upstream_account_id: value.upstream_account_id,
                upstream_account_name: value.upstream_account_name.clone(),
            }
        }
        _ => PromptCacheConversationOperationBindingSnapshot {
            binding_kind: "none".to_string(),
            group_name: None,
            upstream_account_id: None,
            upstream_account_name: None,
        },
    }
}

fn prompt_cache_conversation_bound_proxy_keys_from_row(
    row: Option<&PromptCacheConversationBindingRow>,
) -> Vec<String> {
    row.map(|value| {
        let keys = parse_forward_proxy_keys_json(value.forward_proxy_keys_json.as_deref());
        if keys.is_empty() {
            value
                .forward_proxy_key
                .as_deref()
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(|item| vec![item.to_string()])
                .unwrap_or_default()
        } else {
            keys
        }
    })
    .unwrap_or_default()
}

fn prompt_cache_conversation_policy_changed_fields(
    before: Option<&PromptCacheConversationBindingRow>,
    after: Option<&PromptCacheConversationBindingRow>,
) -> Vec<String> {
    let mut changed_fields = Vec::new();
    let push_if_changed = |changed_fields: &mut Vec<String>, field: &str, changed: bool| -> () {
        if changed {
            changed_fields.push(field.to_string());
        }
    };

    push_if_changed(
        &mut changed_fields,
        "responsesFirstByteTimeoutSecs",
        before.and_then(|row| row.responses_first_byte_timeout_secs)
            != after.and_then(|row| row.responses_first_byte_timeout_secs),
    );
    push_if_changed(
        &mut changed_fields,
        "compactFirstByteTimeoutSecs",
        before.and_then(|row| row.compact_first_byte_timeout_secs)
            != after.and_then(|row| row.compact_first_byte_timeout_secs),
    );
    push_if_changed(
        &mut changed_fields,
        "imageFirstByteTimeoutSecs",
        before.and_then(|row| row.image_first_byte_timeout_secs)
            != after.and_then(|row| row.image_first_byte_timeout_secs),
    );
    push_if_changed(
        &mut changed_fields,
        "responsesStreamTimeoutSecs",
        before.and_then(|row| row.responses_stream_timeout_secs)
            != after.and_then(|row| row.responses_stream_timeout_secs),
    );
    push_if_changed(
        &mut changed_fields,
        "compactStreamTimeoutSecs",
        before.and_then(|row| row.compact_stream_timeout_secs)
            != after.and_then(|row| row.compact_stream_timeout_secs),
    );
    push_if_changed(
        &mut changed_fields,
        "allowSwitchUpstream",
        before.and_then(|row| row.allow_switch_upstream)
            != after.and_then(|row| row.allow_switch_upstream),
    );
    push_if_changed(
        &mut changed_fields,
        "fastModeRewriteMode",
        before.and_then(|row| row.fast_mode_rewrite_mode.as_deref())
            != after.and_then(|row| row.fast_mode_rewrite_mode.as_deref()),
    );
    push_if_changed(
        &mut changed_fields,
        "imageToolRewriteMode",
        before.and_then(|row| row.image_tool_rewrite_mode.as_deref())
            != after.and_then(|row| row.image_tool_rewrite_mode.as_deref()),
    );
    push_if_changed(
        &mut changed_fields,
        "codexImagegenRewriteMode",
        before.and_then(|row| row.codex_imagegen_rewrite_mode.as_deref())
            != after.and_then(|row| row.codex_imagegen_rewrite_mode.as_deref()),
    );
    push_if_changed(
        &mut changed_fields,
        "availableModels",
        before
            .and_then(|row| row.available_models_json.as_deref())
            .and_then(parse_available_models_json)
            != after
                .and_then(|row| row.available_models_json.as_deref())
                .and_then(parse_available_models_json),
    );
    push_if_changed(
        &mut changed_fields,
        "availableModelsMode",
        before.and_then(|row| row.available_models_mode.as_deref())
            != after.and_then(|row| row.available_models_mode.as_deref()),
    );

    let before_proxy_keys = prompt_cache_conversation_bound_proxy_keys_from_row(before);
    let after_proxy_keys = prompt_cache_conversation_bound_proxy_keys_from_row(after);
    push_if_changed(
        &mut changed_fields,
        "forwardProxyKeys",
        before_proxy_keys != after_proxy_keys,
    );
    if before.and_then(|row| row.forward_proxy_key.as_deref())
        != after.and_then(|row| row.forward_proxy_key.as_deref())
        && !changed_fields
            .iter()
            .any(|field| field == "forwardProxyKey")
    {
        changed_fields.push("forwardProxyKey".to_string());
    }

    changed_fields
}

fn prompt_cache_conversation_policy_info_types(changed_fields: &[String]) -> Vec<String> {
    let mut info_types = Vec::new();
    let has_routing = changed_fields.iter().any(|field| {
        matches!(
            field.as_str(),
            "allowSwitchUpstream"
                | "responsesFirstByteTimeoutSecs"
                | "compactFirstByteTimeoutSecs"
                | "imageFirstByteTimeoutSecs"
                | "responsesStreamTimeoutSecs"
                | "compactStreamTimeoutSecs"
        )
    });
    if has_routing {
        info_types.push(PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_ROUTING.to_string());
    }
    let has_forward_proxy = changed_fields
        .iter()
        .any(|field| matches!(field.as_str(), "forwardProxyKey" | "forwardProxyKeys"));
    if has_forward_proxy {
        info_types.push(PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_FORWARD_PROXY.to_string());
    }
    let has_request_rewrite = changed_fields.iter().any(|field| {
        matches!(
            field.as_str(),
            "fastModeRewriteMode"
                | "imageToolRewriteMode"
                | "codexImagegenRewriteMode"
                | "availableModels"
                | "availableModelsMode"
        )
    });
    if has_request_rewrite {
        info_types.push(PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_REQUEST_REWRITE.to_string());
    }
    info_types
}

fn prompt_cache_conversation_operation_headline(action: &str) -> String {
    match action {
        "manualBindingUpdated" => "Manual binding updated",
        "bindingCleared" => "Manual binding cleared",
        "affinityReset" => "Conversation affinity reset",
        "stickyTargetChanged" => "Sticky target changed",
        "stickyTargetCleared" => "Sticky target cleared",
        "stickyMutationSuppressed" => "Sticky mutation suppressed",
        "groupBindingPromoted" => "Group binding promoted",
        "conversationPolicyUpdated" => "Conversation policy updated",
        _ => "Conversation operation updated",
    }
    .to_string()
}

fn parse_prompt_cache_conversation_operation_string_array_json(value: Option<&str>) -> Vec<String> {
    value
        .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
        .map(|values| {
            values
                .into_iter()
                .map(|entry| entry.trim().to_string())
                .filter(|entry| !entry.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn normalize_prompt_cache_conversation_operation_info_type(
    raw: Option<&str>,
) -> Result<Option<String>, ApiError> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    match raw {
        PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_ROUTING
        | PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_FORWARD_PROXY
        | PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_REQUEST_REWRITE => {
            Ok(Some(raw.to_string()))
        }
        _ => Err(ApiError::bad_request(anyhow!(
            "infoType must be one of: routing, forwardProxy, requestRewrite"
        ))),
    }
}

fn normalize_prompt_cache_conversation_operation_routing_scope(
    raw: Option<&str>,
) -> Result<Option<String>, ApiError> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    match raw {
        "all" | "model" => Ok(Some(raw.to_string())),
        _ => Err(ApiError::bad_request(anyhow!(
            "routingScope must be one of: all, model"
        ))),
    }
}

fn normalize_prompt_cache_conversation_operation_routing_model(
    raw: Option<&str>,
) -> Result<Option<String>, ApiError> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    normalize_sticky_model_key(Some(raw))
        .ok_or_else(|| {
            ApiError::bad_request(anyhow!("routingModel must contain a non-empty model name"))
        })
        .map(Some)
}
