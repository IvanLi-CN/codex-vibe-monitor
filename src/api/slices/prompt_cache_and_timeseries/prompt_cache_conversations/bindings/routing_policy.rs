use super::operation_log::{
    AppendPromptCacheConversationOperationEventInput,
    append_prompt_cache_conversation_operation_event,
    load_prompt_cache_conversation_sticky_snapshot,
    prompt_cache_conversation_operation_binding_snapshot_from_row,
    prompt_cache_conversation_operation_headline,
};
use super::{
    ApiError, AppState, ConversationRoutingOverride, PROMPT_CACHE_BINDING_KIND_GROUP,
    PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT,
    PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_ROUTING,
    PROMPT_CACHE_CONVERSATION_OPERATION_ORIGIN_SYSTEM_AUTO, PromptCacheConversationBindingRow,
    UpstreamAccountRow, broadcast_prompt_cache_conversation_changed,
    conversation_routing_override_from_row, format_utc_iso,
    load_prompt_cache_conversation_binding_row,
    load_prompt_cache_conversation_binding_row_executor,
    overwrite_sticky_routes_for_manual_binding,
};
use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use sqlx::{FromRow, Pool, Sqlite};

#[derive(Debug, Clone, FromRow)]
pub(crate) struct PromptCacheEncryptedSessionOwnerRow {
    pub(crate) prompt_cache_key: String,
    pub(crate) owner_upstream_account_id: i64,
    pub(crate) owner_upstream_account_name: Option<String>,
    pub(crate) owner_group_name: Option<String>,
    pub(crate) first_locked_at: String,
    pub(crate) last_confirmed_at: String,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone)]
pub(crate) struct PromptCacheEncryptedSessionRoutingContext {
    pub(crate) owner: PromptCacheEncryptedSessionOwnerRow,
    pub(crate) effective_constraint: PromptCacheConversationBindingConstraint,
    pub(crate) manual_override_active: bool,
}
#[derive(Debug, Clone)]
pub(crate) enum PromptCacheConversationBindingConstraint {
    Group(String),
    UpstreamAccount(i64),
}

impl PromptCacheConversationBindingConstraint {
    pub(crate) fn accepts_row(&self, row: &UpstreamAccountRow) -> bool {
        match self {
            Self::Group(group_name) => row
                .normalized_group_name()
                .is_some_and(|value| value == group_name),
            Self::UpstreamAccount(account_id) => row.id() == *account_id,
        }
    }
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

    #[derive(Debug, FromRow)]
    struct PromotionTargetRow {
        group_name: Option<String>,
    }

    let Some(target_row) = sqlx::query_as::<_, PromotionTargetRow>(
        r#"
        SELECT group_name
        FROM pool_upstream_accounts
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(upstream_account_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(false);
    };

    let target_group_name = target_row
        .group_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if target_group_name != Some(bound_group_name) {
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
                    &current_binding,
                )),
            ),
            binding_after: Some(
                prompt_cache_conversation_operation_binding_snapshot_from_row(
                    promoted_binding.as_ref(),
                ),
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

pub(crate) async fn delete_prompt_cache_encrypted_session_owner(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
) -> Result<()> {
    delete_prompt_cache_encrypted_session_owner_executor(pool, prompt_cache_key).await
}

pub(crate) async fn delete_prompt_cache_encrypted_session_owner_executor<'e, E>(
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
