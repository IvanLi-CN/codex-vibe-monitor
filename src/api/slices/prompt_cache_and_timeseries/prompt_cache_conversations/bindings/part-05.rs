pub(crate) fn parse_available_models_json(value: &str) -> Option<Vec<String>> {
    parse_available_models_json_with_invalid(value).0
}

pub(crate) fn parse_available_models_json_with_invalid(value: &str) -> (Option<Vec<String>>, bool) {
    let values = match serde_json::from_str::<Vec<String>>(value) {
        Ok(values) => values,
        Err(_) => return (None, true),
    };
    let invalid_entry = values.iter().any(|value| value.trim().is_empty());
    let normalized = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    (Some(normalized), invalid_entry)
}

pub(crate) fn parse_forward_proxy_keys_json(value: Option<&str>) -> Vec<String> {
    value
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .map(normalize_bound_proxy_keys)
        .unwrap_or_default()
}

pub(crate) fn conversation_routing_override_from_row(
    row: &PromptCacheConversationBindingRow,
) -> Option<ConversationRoutingOverride> {
    let forward_proxy_keys = parse_forward_proxy_keys_json(row.forward_proxy_keys_json.as_deref());
    let override_policy = ConversationRoutingOverride {
        allow_switch_upstream: row.allow_switch_upstream.map(|value| value != 0),
        fast_mode_rewrite_mode: row
            .fast_mode_rewrite_mode
            .as_deref()
            .map(parse_fast_mode_rewrite_mode_lossy),
        image_tool_rewrite_mode: row
            .image_tool_rewrite_mode
            .as_deref()
            .map(ImageToolRewriteMode::from_str),
        codex_imagegen_rewrite_mode: row
            .codex_imagegen_rewrite_mode
            .as_deref()
            .map(CodexImagegenRewriteMode::from_str),
        available_models: row
            .available_models_json
            .as_deref()
            .map(parse_available_models_json_with_invalid)
            .and_then(|(models, _)| models),
        available_models_invalid: row
            .available_models_json
            .as_deref()
            .is_some_and(|raw| parse_available_models_json_with_invalid(raw).1),
        available_models_mode: row
            .available_models_mode
            .as_deref()
            .map(|value| AvailableModelsMode::from_str(Some(value)))
            .or_else(|| {
                row.available_models_json
                    .as_ref()
                    .map(|_| AvailableModelsMode::Allowlist)
            }),
        forward_proxy_key: row
            .forward_proxy_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        forward_proxy_keys,
        forward_proxy_scope_key: format!("conversation:{}", row.prompt_cache_key),
    };
    override_policy
        .has_policy_override()
        .then_some(override_policy)
}

pub(crate) async fn load_prompt_cache_conversation_routing_override(
    pool: &Pool<Sqlite>,
    prompt_cache_key: Option<&str>,
) -> Result<Option<ConversationRoutingOverride>> {
    let Some(prompt_cache_key) = prompt_cache_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    Ok(
        load_prompt_cache_conversation_binding_row(pool, prompt_cache_key)
            .await?
            .as_ref()
            .and_then(conversation_routing_override_from_row),
    )
}

pub(crate) async fn load_prompt_cache_encrypted_session_owner_row_executor<'e, E>(
    executor: E,
    prompt_cache_key: &str,
) -> Result<Option<PromptCacheEncryptedSessionOwnerRow>>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    sqlx::query_as::<_, PromptCacheEncryptedSessionOwnerRow>(
        r#"
        SELECT
            owner.prompt_cache_key,
            owner.owner_upstream_account_id,
            account.display_name AS owner_upstream_account_name,
            account.group_name AS owner_group_name,
            owner.first_locked_at,
            owner.last_confirmed_at,
            owner.updated_at
        FROM prompt_cache_encrypted_session_owners AS owner
        LEFT JOIN pool_upstream_accounts AS account
          ON account.id = owner.owner_upstream_account_id
        WHERE owner.prompt_cache_key = ?1
        LIMIT 1
        "#,
    )
    .bind(prompt_cache_key)
    .fetch_optional(executor)
    .await
    .map_err(Into::into)
}

pub(crate) async fn load_prompt_cache_encrypted_session_owner_row(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
) -> Result<Option<PromptCacheEncryptedSessionOwnerRow>> {
    load_prompt_cache_encrypted_session_owner_row_executor(pool, prompt_cache_key).await
}

pub(crate) async fn load_prompt_cache_encrypted_session_owner_row_if_enabled(
    state: &AppState,
    prompt_cache_key: &str,
) -> Result<Option<PromptCacheEncryptedSessionOwnerRow>> {
    if !state
        .proxy_model_settings
        .read()
        .await
        .encrypted_session_owner_routing_enabled
    {
        return Ok(None);
    }
    load_prompt_cache_encrypted_session_owner_row(&state.pool, prompt_cache_key).await
}

pub(crate) async fn load_prompt_cache_encrypted_session_owner_account_id(
    pool: &Pool<Sqlite>,
    prompt_cache_key: Option<&str>,
) -> Result<Option<i64>> {
    let Some(prompt_cache_key) = prompt_cache_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    Ok(
        load_prompt_cache_encrypted_session_owner_row(pool, prompt_cache_key)
            .await?
            .map(|row| row.owner_upstream_account_id),
    )
}

pub(crate) fn manual_binding_overrides_encrypted_owner(
    binding_row: &PromptCacheConversationBindingRow,
    owner: &PromptCacheEncryptedSessionOwnerRow,
) -> bool {
    if binding_row.updated_at.as_str() < owner.first_locked_at.as_str() {
        return false;
    }
    match binding_row.binding_kind.as_str() {
        PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT => {
            binding_row.upstream_account_id != Some(owner.owner_upstream_account_id)
        }
        PROMPT_CACHE_BINDING_KIND_GROUP => binding_row
            .group_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some(),
        _ => false,
    }
}

pub(crate) fn binding_constraint_accepts_upstream_account_id(
    constraint: &PromptCacheConversationBindingConstraint,
    account_id: i64,
    account_group_name: Option<&str>,
) -> bool {
    match constraint {
        PromptCacheConversationBindingConstraint::Group(group_name) => account_group_name
            .map(str::trim)
            .is_some_and(|value| value == group_name),
        PromptCacheConversationBindingConstraint::UpstreamAccount(bound_id) => {
            *bound_id == account_id
        }
    }
}

pub(crate) async fn resolve_prompt_cache_encrypted_session_routing_context(
    pool: &Pool<Sqlite>,
    prompt_cache_key: Option<&str>,
    _request_contains_encrypted_content: bool,
) -> Result<Option<PromptCacheEncryptedSessionRoutingContext>> {
    let Some(prompt_cache_key) = prompt_cache_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };

    let owner = load_prompt_cache_encrypted_session_owner_row(pool, prompt_cache_key).await?;
    let binding_row = load_prompt_cache_conversation_binding_row(pool, prompt_cache_key).await?;

    match owner {
        Some(owner) => {
            let override_constraint = if binding_row
                .as_ref()
                .is_some_and(|row| manual_binding_overrides_encrypted_owner(row, &owner))
            {
                load_prompt_cache_conversation_binding_constraint(pool, Some(prompt_cache_key))
                    .await?
            } else {
                None
            };
            let manual_override_active = override_constraint.is_some();
            let owner_account_id = owner.owner_upstream_account_id;
            Ok(Some(PromptCacheEncryptedSessionRoutingContext {
                owner,
                effective_constraint: override_constraint.unwrap_or(
                    PromptCacheConversationBindingConstraint::UpstreamAccount(owner_account_id),
                ),
                manual_override_active,
            }))
        }
        None => Ok(None),
    }
}

pub(crate) async fn upsert_prompt_cache_encrypted_session_owner_executor<'e, E>(
    executor: E,
    prompt_cache_key: &str,
    owner_upstream_account_id: i64,
) -> Result<()>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        r#"
        INSERT INTO prompt_cache_encrypted_session_owners (
            prompt_cache_key,
            owner_upstream_account_id,
            first_locked_at,
            last_confirmed_at,
            updated_at
        )
        VALUES (?1, ?2, datetime('now'), datetime('now'), datetime('now'))
        ON CONFLICT(prompt_cache_key) DO UPDATE SET
            owner_upstream_account_id = excluded.owner_upstream_account_id,
            last_confirmed_at = excluded.last_confirmed_at,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(prompt_cache_key)
    .bind(owner_upstream_account_id)
    .execute(executor)
    .await?;
    Ok(())
}

pub(crate) async fn upsert_prompt_cache_encrypted_session_owner(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
    owner_upstream_account_id: i64,
) -> Result<()> {
    upsert_prompt_cache_encrypted_session_owner_executor(
        pool,
        prompt_cache_key,
        owner_upstream_account_id,
    )
    .await
}

pub(crate) async fn confirm_prompt_cache_encrypted_session_owner_success(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
    owner_upstream_account_id: i64,
) -> Result<bool> {
    let prompt_cache_key = prompt_cache_key.trim();
    if prompt_cache_key.is_empty() {
        return Ok(false);
    }

    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(conn.as_mut())
        .await
        .context("failed to acquire encrypted session owner write lock")?;

    let outcome: Result<bool> = async {
        let owner_row =
            load_prompt_cache_encrypted_session_owner_row_executor(conn.as_mut(), prompt_cache_key)
                .await?;
        let binding_row =
            load_prompt_cache_conversation_binding_row_executor(conn.as_mut(), prompt_cache_key)
                .await?;

        let should_update = match owner_row.as_ref() {
            None => true,
            Some(owner) if owner.owner_upstream_account_id == owner_upstream_account_id => true,
            Some(owner) => {
                let override_constraint = binding_row.as_ref().and_then(|row| {
                    manual_binding_overrides_encrypted_owner(row, owner).then_some(row)
                });
                match override_constraint {
                    Some(row) => match row.binding_kind.as_str() {
                        PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT => {
                            row.upstream_account_id == Some(owner_upstream_account_id)
                        }
                        PROMPT_CACHE_BINDING_KIND_GROUP => {
                            let account_group_name: Option<String> = sqlx::query_scalar(
                                r#"
                                SELECT group_name
                                FROM pool_upstream_accounts
                                WHERE id = ?1
                                LIMIT 1
                                "#,
                            )
                            .bind(owner_upstream_account_id)
                            .fetch_optional(conn.as_mut())
                            .await?
                            .flatten();
                            row.group_name
                                .as_deref()
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                                == account_group_name
                                    .as_deref()
                                    .map(str::trim)
                                    .filter(|value| !value.is_empty())
                        }
                        _ => false,
                    },
                    None => false,
                }
            }
        };

        if !should_update {
            return Ok(false);
        }

        upsert_prompt_cache_encrypted_session_owner_executor(
            conn.as_mut(),
            prompt_cache_key,
            owner_upstream_account_id,
        )
        .await?;
        Ok(true)
    }
    .await;

    match outcome {
        Ok(should_update) => {
            sqlx::query("COMMIT")
                .execute(conn.as_mut())
                .await
                .context("failed to commit encrypted session owner update")?;
            Ok(should_update)
        }
        Err(err) => {
            let _ = sqlx::query("ROLLBACK").execute(conn.as_mut()).await;
            Err(err)
        }
    }
}

async fn load_promotion_target_group_name(
    pool: &Pool<Sqlite>,
    upstream_account_id: i64,
) -> Result<Option<String>> {
    #[derive(Debug, FromRow)]
    struct PromotionTargetRow {
        group_name: Option<String>,
    }
    Ok(sqlx::query_as::<_, PromotionTargetRow>(
        "SELECT group_name FROM pool_upstream_accounts WHERE id = ?1 LIMIT 1",
    )
    .bind(upstream_account_id)
    .fetch_optional(pool)
    .await?
    .and_then(|row| {
        row.group_name
            .filter(|value| !value.trim().is_empty())
            .map(|value| value.trim().to_string())
    }))
}

async fn append_promotion_operation_events(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
    current_binding: &PromptCacheConversationBindingRow,
    promoted_binding: Option<&PromptCacheConversationBindingRow>,
    sticky_before: Option<PromptCacheConversationOperationStickySnapshot>,
    sticky_after: Option<PromptCacheConversationOperationStickySnapshot>,
    now_iso: String,
) -> Result<()> {
    append_prompt_cache_conversation_operation_event(
        pool,
        AppendPromptCacheConversationOperationEventInput {
            prompt_cache_key: prompt_cache_key.to_string(),
            action: "groupBindingPromoted".to_string(),
            origin: PROMPT_CACHE_CONVERSATION_OPERATION_ORIGIN_SYSTEM_AUTO.to_string(),
            info_types: vec![PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_ROUTING.to_string()],
            occurred_at: now_iso.clone(),
            headline: prompt_cache_conversation_operation_headline("groupBindingPromoted"),
            changed_fields: vec!["bindingKind".to_string()],
            binding_before: Some(
                prompt_cache_conversation_operation_binding_snapshot_from_row(Some(
                    current_binding,
                )),
            ),
            binding_after: Some(
                prompt_cache_conversation_operation_binding_snapshot_from_row(promoted_binding),
            ),
            sticky_before: sticky_before.clone(),
            sticky_after: sticky_after.clone(),
            invoke_id: None,
        },
    )
    .await?;
    if sticky_before != sticky_after
        && let Some(sticky_after) = sticky_after
    {
        append_prompt_cache_conversation_operation_event(
            pool,
            AppendPromptCacheConversationOperationEventInput {
                prompt_cache_key: prompt_cache_key.to_string(),
                action: "stickyTargetChanged".to_string(),
                origin: PROMPT_CACHE_CONVERSATION_OPERATION_ORIGIN_SYSTEM_AUTO.to_string(),
                info_types: vec![PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_ROUTING.to_string()],
                occurred_at: now_iso,
                headline: prompt_cache_conversation_operation_headline("stickyTargetChanged"),
                changed_fields: vec!["stickyTarget".to_string()],
                binding_before: None,
                binding_after: None,
                sticky_before,
                sticky_after: Some(sticky_after),
                invoke_id: None,
            },
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn promote_prompt_cache_group_binding_to_upstream_account(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
    upstream_account_id: i64,
) -> Result<bool> {
    let Some(current_binding) =
        load_prompt_cache_conversation_binding_row(pool, prompt_cache_key).await?
    else {
        return Ok(false);
    };
    if current_binding.binding_kind != PROMPT_CACHE_BINDING_KIND_GROUP {
        return Ok(false);
    }
    let Some(bound_group_name) = current_binding
        .group_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(false);
    };

    let Some(target_group_name) =
        load_promotion_target_group_name(pool, upstream_account_id).await?
    else {
        return Ok(false);
    };
    if target_group_name != bound_group_name {
        return Ok(false);
    }

    let sticky_before =
        load_prompt_cache_conversation_sticky_snapshot(pool, prompt_cache_key).await?;
    let update_result = sqlx::query(
        r#"
        UPDATE prompt_cache_conversation_bindings
        SET binding_kind = ?2,
            group_name = NULL,
            upstream_account_id = ?3,
            updated_at = datetime('now')
        WHERE prompt_cache_key = ?1
          AND binding_kind = ?4
          AND group_name = ?5
        "#,
    )
    .bind(prompt_cache_key)
    .bind(PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT)
    .bind(upstream_account_id)
    .bind(PROMPT_CACHE_BINDING_KIND_GROUP)
    .bind(bound_group_name)
    .execute(pool)
    .await?;
    if update_result.rows_affected() == 0 {
        return Ok(false);
    }

    let now_iso = format_utc_iso(Utc::now());
    overwrite_sticky_routes_for_manual_binding(
        pool,
        prompt_cache_key,
        upstream_account_id,
        &now_iso,
    )
    .await?;
    let promoted_binding =
        load_prompt_cache_conversation_binding_row(pool, prompt_cache_key).await?;
    let sticky_after =
        load_prompt_cache_conversation_sticky_snapshot(pool, prompt_cache_key).await?;
    append_promotion_operation_events(
        pool,
        prompt_cache_key,
        &current_binding,
        promoted_binding.as_ref(),
        sticky_before,
        sticky_after,
        now_iso,
    )
    .await?;
    Ok(true)
}

pub(crate) async fn promote_prompt_cache_group_binding_to_upstream_account_and_broadcast(
    state: &AppState,
    prompt_cache_key: &str,
    upstream_account_id: i64,
) -> Result<()> {
    if promote_prompt_cache_group_binding_to_upstream_account(
        &state.pool,
        prompt_cache_key,
        upstream_account_id,
    )
    .await?
    {
        broadcast_prompt_cache_conversation_changed(state, prompt_cache_key).await;
    }
    Ok(())
}

pub(crate) async fn resolve_prompt_cache_effective_routing_constraint(
    pool: &Pool<Sqlite>,
    prompt_cache_key: Option<&str>,
    request_contains_encrypted_content: bool,
    encrypted_session_owner_routing_enabled: bool,
) -> Result<(Option<PromptCacheConversationBindingConstraint>, bool)> {
    if encrypted_session_owner_routing_enabled
        && let Some(context) = resolve_prompt_cache_encrypted_session_routing_context(
            pool,
            prompt_cache_key,
            request_contains_encrypted_content,
        )
        .await?
    {
        // Clearing a manual binding only removes the dangerous override intent.
        // Automatic routing still stays on the encrypted-session owner until a
        // different target actually succeeds and becomes the new owner.
        let owner_auto_guard_active =
            context.owner.owner_upstream_account_id > 0 && !context.manual_override_active;
        return Ok((Some(context.effective_constraint), owner_auto_guard_active));
    }

    Ok((
        load_prompt_cache_conversation_binding_constraint(pool, prompt_cache_key).await?,
        false,
    ))
}

pub(crate) async fn load_prompt_cache_conversation_binding_constraint(
    pool: &Pool<Sqlite>,
    prompt_cache_key: Option<&str>,
) -> Result<Option<PromptCacheConversationBindingConstraint>> {
    let Some(prompt_cache_key) = prompt_cache_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let Some(row) = load_prompt_cache_conversation_binding_row(pool, prompt_cache_key).await?
    else {
        return Ok(None);
    };
    Ok(match row.binding_kind.as_str() {
        PROMPT_CACHE_BINDING_KIND_GROUP => row
            .group_name
            .map(PromptCacheConversationBindingConstraint::Group),
        PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT => row
            .upstream_account_id
            .map(PromptCacheConversationBindingConstraint::UpstreamAccount),
        _ => None,
    })
}

pub(crate) async fn ensure_group_binding_target(
    pool: &Pool<Sqlite>,
    group_name: &str,
) -> Result<(), ApiError> {
    let account_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_accounts
        WHERE TRIM(COALESCE(group_name, '')) = ?1
          AND provider = 'codex'
          AND enabled != 0
          AND status = 'active'
          AND encrypted_credentials IS NOT NULL
        "#,
    )
    .bind(group_name)
    .fetch_one(pool)
    .await?;
    if account_count <= 0 {
        return Err(ApiError::bad_request(anyhow!(
            "groupName must reference an existing upstream account group"
        )));
    }
    Ok(())
}

pub(crate) async fn ensure_upstream_account_binding_target(
    pool: &Pool<Sqlite>,
    upstream_account_id: i64,
) -> Result<String, ApiError> {
    #[derive(Debug, FromRow)]
    struct AccountTargetRow {
        display_name: String,
        provider: String,
        enabled: i64,
        status: String,
        encrypted_credentials: Option<String>,
    }

    let Some(row) = sqlx::query_as::<_, AccountTargetRow>(
        r#"
        SELECT display_name, provider, enabled, status, encrypted_credentials
        FROM pool_upstream_accounts
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(upstream_account_id)
    .fetch_optional(pool)
    .await?
    else {
        return Err(ApiError::bad_request(anyhow!(
            "upstreamAccountId must reference an existing upstream account"
        )));
    };
    if row.provider != "codex" {
        return Err(ApiError::bad_request(anyhow!(
            "upstreamAccountId must reference an account-pool upstream account"
        )));
    }
    if row.enabled == 0 || row.status != "active" || row.encrypted_credentials.is_none() {
        return Err(ApiError::bad_request(anyhow!(
            "upstreamAccountId must reference a selectable account-pool upstream account"
        )));
    }
    Ok(row.display_name)
}

pub(crate) async fn normalize_forward_proxy_key_patch(
    state: &AppState,
    value: PatchField<String>,
) -> Result<PatchField<String>, ApiError> {
    let value = match value {
        PatchField::Missing => return Ok(PatchField::Missing),
        PatchField::Null => return Ok(PatchField::Null),
        PatchField::Value(value) => value,
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(PatchField::Null);
    }
    let manager = state.forward_proxy.lock().await;
    let Some(canonical) = manager.canonicalize_bound_proxy_key(value, None) else {
        return Err(ApiError::bad_request(anyhow!(
            "forwardProxyKey must reference an existing forward proxy binding node"
        )));
    };
    if !manager
        .binding_nodes()
        .into_iter()
        .any(|node| node.key == canonical && node.selectable)
    {
        return Err(ApiError::bad_request(anyhow!(
            "forwardProxyKey must reference a selectable forward proxy binding node"
        )));
    }
    Ok(PatchField::Value(canonical))
}

pub(crate) async fn normalize_forward_proxy_keys_patch(
    state: &AppState,
    value: PatchField<Vec<String>>,
) -> Result<PatchField<Vec<String>>, ApiError> {
    let values = match value {
        PatchField::Missing => return Ok(PatchField::Missing),
        PatchField::Null => return Ok(PatchField::Null),
        PatchField::Value(values) => values,
    };
    let normalized = normalize_bound_proxy_keys(values);
    if normalized.is_empty() {
        return Ok(PatchField::Null);
    }
    let canonical = canonicalize_forward_proxy_bound_keys(state, &normalized).await?;
    let has_selectable = {
        let manager = state.forward_proxy.lock().await;
        manager.has_selectable_bound_proxy_keys(&canonical)
    };
    if !has_selectable {
        return Err(ApiError::bad_request(anyhow!(
            "forwardProxyKeys must contain at least one selectable forward proxy binding node"
        )));
    }
    Ok(PatchField::Value(canonical))
}

pub(crate) fn next_optional_patch_value<T: Clone>(
    incoming: PatchField<T>,
    current: Option<T>,
) -> Option<T> {
    match incoming {
        PatchField::Missing => current,
        PatchField::Null => None,
        PatchField::Value(value) => Some(value),
    }
}

pub(crate) fn next_optional_value<T: Clone>(
    incoming: Option<Option<T>>,
    current: Option<T>,
) -> Option<T> {
    incoming.unwrap_or_else(|| current.map(Some).unwrap_or(None))
}

fn normalized_prompt_cache_conversation_keys(
    raw_keys: Vec<String>,
) -> Result<Vec<String>, ApiError> {
    let mut seen = HashSet::new();
    let mut normalized_keys = Vec::with_capacity(raw_keys.len());
    for raw_key in raw_keys {
        let normalized_key = normalize_prompt_cache_conversation_key(&raw_key)?;
        if seen.insert(normalized_key.clone()) {
            normalized_keys.push(normalized_key);
        }
    }
    if normalized_keys.is_empty() {
        return Err(ApiError::bad_request(anyhow!(
            "promptCacheKeys must contain at least one key"
        )));
    }
    Ok(normalized_keys)
}

pub(crate) async fn load_prompt_cache_conversation_binding_response_for_key(
    state: &AppState,
    prompt_cache_key: String,
) -> Result<PromptCacheConversationBindingResponse, ApiError> {
    let owner =
        load_prompt_cache_encrypted_session_owner_row_if_enabled(state, &prompt_cache_key).await?;
    let response = match load_prompt_cache_conversation_binding_row(&state.pool, &prompt_cache_key)
        .await?
    {
        Some(row) => binding_response_from_row(state, &state.config, row, owner.as_ref()).await?,
        None => apply_owner_to_none_response(
            binding_response_for_none(state, &state.config, prompt_cache_key, owner.as_ref())
                .await?,
            owner.as_ref(),
        ),
    };
    Ok(response)
}

async fn delete_prompt_cache_encrypted_session_owner(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
) -> Result<()> {
    delete_prompt_cache_encrypted_session_owner_executor(pool, prompt_cache_key).await
}

async fn delete_prompt_cache_encrypted_session_owner_executor<'e, E>(
    executor: E,
    prompt_cache_key: &str,
) -> Result<()>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    sqlx::query("DELETE FROM prompt_cache_encrypted_session_owners WHERE prompt_cache_key = ?1")
        .bind(prompt_cache_key)
        .execute(executor)
        .await?;
    Ok(())
}

async fn clear_prompt_cache_conversation_affinity(
    state: &AppState,
    prompt_cache_key: &str,
    origin: &str,
) -> Result<PromptCacheConversationBindingResponse, ApiError> {
    let _write_guard = PROMPT_CACHE_BINDING_WRITE_LOCK.lock().await;
    let mut conn = state.pool.acquire().await?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(conn.as_mut())
        .await
        .context("failed to acquire prompt cache affinity reset write lock")?;

    let reset_outcome: Result<(), ApiError> = async {
        let existing_row =
            load_prompt_cache_conversation_binding_row_executor(conn.as_mut(), prompt_cache_key)
                .await?;
        let sticky_before = load_prompt_cache_conversation_sticky_snapshot_executor(
            conn.as_mut(),
            prompt_cache_key,
        )
        .await?;
        let sticky_routes_before =
            load_prompt_cache_conversation_sticky_routes_executor(conn.as_mut(), prompt_cache_key)
                .await?;
        let now_iso = format_utc_iso(Utc::now());
        bump_sticky_affinity_generation_executor(conn.as_mut(), prompt_cache_key, &now_iso).await?;
        sqlx::query("DELETE FROM prompt_cache_conversation_bindings WHERE prompt_cache_key = ?1")
            .bind(prompt_cache_key)
            .execute(conn.as_mut())
            .await?;
        delete_sticky_route_executor(conn.as_mut(), prompt_cache_key).await?;
        delete_sticky_model_routes_executor(conn.as_mut(), prompt_cache_key).await?;
        delete_prompt_cache_encrypted_session_owner_executor(conn.as_mut(), prompt_cache_key)
            .await?;
        let sticky_transitions =
            prompt_cache_conversation_sticky_transitions(&sticky_routes_before, &[]);
        append_prompt_cache_conversation_operation_event_with_sticky_transitions_executor(
            conn.as_mut(),
            AppendPromptCacheConversationOperationEventInput {
                prompt_cache_key: prompt_cache_key.to_string(),
                action: "affinityReset".to_string(),
                origin: origin.to_string(),
                info_types: vec![PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_ROUTING.to_string()],
                occurred_at: now_iso.clone(),
                headline: prompt_cache_conversation_operation_headline("affinityReset"),
                changed_fields: vec!["bindingKind".to_string()],
                binding_before: Some(
                    prompt_cache_conversation_operation_binding_snapshot_from_row(
                        existing_row.as_ref(),
                    ),
                ),
                binding_after: Some(
                    prompt_cache_conversation_operation_binding_snapshot_from_row(None),
                ),
                sticky_before: sticky_before.clone(),
                sticky_after: None,
                invoke_id: None,
            },
            &sticky_transitions,
        )
        .await?;
        Ok(())
    }
    .await;

    match reset_outcome {
        Ok(()) => {
            sqlx::query("COMMIT")
                .execute(conn.as_mut())
                .await
                .context("failed to commit prompt cache affinity reset transaction")?;
        }
        Err(error) => {
            let _ = sqlx::query("ROLLBACK").execute(conn.as_mut()).await;
            return Err(error);
        }
    }
    drop(conn);
    broadcast_prompt_cache_conversation_changed(state, prompt_cache_key).await;
    load_prompt_cache_conversation_binding_response_for_key(state, prompt_cache_key.to_string())
        .await
}
