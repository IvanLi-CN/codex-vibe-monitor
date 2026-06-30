use super::*;
const PROMPT_CACHE_BINDING_KIND_GROUP: &str = "group";
const PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT: &str = "upstream_account";
const PROMPT_CACHE_BINDING_KIND_NONE: &str = "none";

#[derive(Debug, Clone, FromRow)]
pub(crate) struct PromptCacheConversationBindingRow {
    pub(crate) prompt_cache_key: String,
    pub(crate) binding_kind: String,
    pub(crate) group_name: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) responses_first_byte_timeout_secs: Option<i64>,
    pub(crate) compact_first_byte_timeout_secs: Option<i64>,
    pub(crate) responses_stream_timeout_secs: Option<i64>,
    pub(crate) compact_stream_timeout_secs: Option<i64>,
    pub(crate) updated_at: String,
}

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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationBindingResponse {
    pub(crate) prompt_cache_key: String,
    pub(crate) binding_kind: String,
    pub(crate) group_name: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) has_encrypted_session_owner: bool,
    pub(crate) encrypted_owner_account_id: Option<i64>,
    pub(crate) encrypted_owner_account_name: Option<String>,
    pub(crate) encrypted_owner_group_name: Option<String>,
    pub(crate) timeouts: RoutingTimeoutSettings,
    pub(crate) timeout_field_sources: RoutingTimeoutFieldSources,
    pub(crate) updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdatePromptCacheConversationBindingRequest {
    binding_kind: String,
    group_name: Option<String>,
    upstream_account_id: Option<i64>,
    #[serde(default)]
    timeouts: Option<UpdateRoutingTimeoutSettingsRequest>,
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

pub(crate) fn normalize_prompt_cache_conversation_key(raw: &str) -> Result<String, ApiError> {
    let normalized = raw.trim();
    if normalized.is_empty() {
        return Err(ApiError::bad_request(anyhow!(
            "prompt cache key is required"
        )));
    }
    Ok(normalized.to_string())
}

async fn binding_response_for_none(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    prompt_cache_key: String,
    owner: Option<&PromptCacheEncryptedSessionOwnerRow>,
) -> Result<PromptCacheConversationBindingResponse> {
    let (timeouts, timeout_field_sources) = if let Some(owner) = owner {
        let (timeouts, sources, _) = load_effective_request_path_timeouts_for_account(
            pool,
            config,
            owner.owner_upstream_account_id,
            Some(prompt_cache_key.as_str()),
        )
        .await?;
        (timeouts, sources)
    } else {
        let (timeouts, sources, _) = load_effective_request_path_timeouts_for_group_and_conversation(
            pool,
            config,
            None,
            Some(prompt_cache_key.as_str()),
        )
        .await?;
        (timeouts, sources)
    };

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
        timeouts,
        timeout_field_sources,
        updated_at: None,
    })
}

async fn binding_response_from_row(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    row: PromptCacheConversationBindingRow,
    owner: Option<&PromptCacheEncryptedSessionOwnerRow>,
) -> Result<PromptCacheConversationBindingResponse> {
    let (timeouts, timeout_field_sources) = if row.binding_kind == PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT {
        if let Some(account_id) = row.upstream_account_id {
            let (timeouts, sources, _) = load_effective_request_path_timeouts_for_account(
                pool,
                config,
                account_id,
                Some(row.prompt_cache_key.as_str()),
            )
            .await?;
            (timeouts, sources)
        } else {
            let (timeouts, sources, _) = load_effective_request_path_timeouts_for_group_and_conversation(
                pool,
                config,
                None,
                Some(row.prompt_cache_key.as_str()),
            )
            .await?;
            (timeouts, sources)
        }
    } else if row.binding_kind == PROMPT_CACHE_BINDING_KIND_GROUP {
        let (timeouts, sources, _) = load_effective_request_path_timeouts_for_group_and_conversation(
            pool,
            config,
            row.group_name.as_deref(),
            Some(row.prompt_cache_key.as_str()),
        )
        .await?;
        (timeouts, sources)
    } else if let Some(owner) = owner {
        let (timeouts, sources, _) = load_effective_request_path_timeouts_for_account(
            pool,
            config,
            owner.owner_upstream_account_id,
            Some(row.prompt_cache_key.as_str()),
        )
        .await?;
        (timeouts, sources)
    } else {
        let (timeouts, sources, _) = load_effective_request_path_timeouts_for_group_and_conversation(
            pool,
            config,
            None,
            Some(row.prompt_cache_key.as_str()),
        )
        .await?;
        (timeouts, sources)
    };

    Ok(PromptCacheConversationBindingResponse {
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
        timeouts,
        timeout_field_sources,
        updated_at: Some(row.updated_at),
    })
}

fn apply_owner_to_none_response(
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

async fn load_prompt_cache_conversation_binding_row_executor<'e, E>(
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
            binding.responses_stream_timeout_secs,
            binding.compact_stream_timeout_secs,
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

async fn load_prompt_cache_encrypted_session_owner_row_executor<'e, E>(
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
    Ok(load_prompt_cache_encrypted_session_owner_row(pool, prompt_cache_key)
        .await?
        .map(|row| row.owner_upstream_account_id))
}

fn manual_binding_override_is_newer_than_owner(
    binding_row: &PromptCacheConversationBindingRow,
    owner: &PromptCacheEncryptedSessionOwnerRow,
) -> bool {
    if binding_row.binding_kind == PROMPT_CACHE_BINDING_KIND_NONE {
        return false;
    }
    match binding_row.updated_at.as_str().cmp(owner.updated_at.as_str()) {
        std::cmp::Ordering::Greater => match binding_row.binding_kind.as_str() {
            PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT => {
                binding_row.upstream_account_id != Some(owner.owner_upstream_account_id)
            }
            PROMPT_CACHE_BINDING_KIND_GROUP => true,
            _ => true,
        },
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => match binding_row.binding_kind.as_str() {
            PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT => {
                binding_row.upstream_account_id != Some(owner.owner_upstream_account_id)
            }
            PROMPT_CACHE_BINDING_KIND_GROUP => true,
            _ => true,
        },
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
                .is_some_and(|row| manual_binding_override_is_newer_than_owner(row, &owner))
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

async fn upsert_prompt_cache_encrypted_session_owner_executor<'e, E>(
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
                    manual_binding_override_is_newer_than_owner(row, owner).then_some(row)
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
) -> Result<()> {
    let Some(current_binding) =
        load_prompt_cache_conversation_binding_row(pool, prompt_cache_key).await?
    else {
        return Ok(());
    };
    if current_binding.binding_kind != PROMPT_CACHE_BINDING_KIND_GROUP {
        return Ok(());
    }
    let Some(bound_group_name) = current_binding
        .group_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
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
        return Ok(());
    };

    let target_group_name = target_row
        .group_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if target_group_name != Some(bound_group_name) {
        return Ok(());
    }

    sqlx::query(
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

    let now_iso = format_utc_iso(Utc::now());
    upsert_sticky_route(pool, prompt_cache_key, upstream_account_id, &now_iso).await?;
    Ok(())
}

pub(crate) async fn resolve_prompt_cache_effective_routing_constraint(
    pool: &Pool<Sqlite>,
    prompt_cache_key: Option<&str>,
    request_contains_encrypted_content: bool,
) -> Result<(Option<PromptCacheConversationBindingConstraint>, bool)> {
    if let Some(context) = resolve_prompt_cache_encrypted_session_routing_context(
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

async fn ensure_group_binding_target(pool: &Pool<Sqlite>, group_name: &str) -> Result<(), ApiError> {
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

async fn ensure_upstream_account_binding_target(
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

pub(crate) async fn get_prompt_cache_conversation_binding(
    State(state): State<Arc<AppState>>,
    AxumPath(encoded_prompt_cache_key): AxumPath<String>,
) -> Result<Json<PromptCacheConversationBindingResponse>, ApiError> {
    let prompt_cache_key = normalize_prompt_cache_conversation_key(&encoded_prompt_cache_key)?;
    let owner =
        load_prompt_cache_encrypted_session_owner_row(&state.pool, &prompt_cache_key).await?;
    let response = match load_prompt_cache_conversation_binding_row(&state.pool, &prompt_cache_key).await? {
        Some(row) => binding_response_from_row(&state.pool, &state.config, row, owner.as_ref()).await?,
        None => apply_owner_to_none_response(
            binding_response_for_none(&state.pool, &state.config, prompt_cache_key, owner.as_ref()).await?,
            owner.as_ref(),
        ),
    };
    Ok(Json(response))
}

pub(crate) async fn patch_prompt_cache_conversation_binding(
    State(state): State<Arc<AppState>>,
    AxumPath(encoded_prompt_cache_key): AxumPath<String>,
    Json(payload): Json<UpdatePromptCacheConversationBindingRequest>,
) -> Result<Json<PromptCacheConversationBindingResponse>, ApiError> {
    let prompt_cache_key = normalize_prompt_cache_conversation_key(&encoded_prompt_cache_key)?;
    let binding_kind = payload.binding_kind.trim();
    let group_name = payload
        .group_name
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let upstream_account_id = payload.upstream_account_id;
    if group_name.is_some() && upstream_account_id.is_some() {
        return Err(ApiError::bad_request(anyhow!(
            "groupName and upstreamAccountId are mutually exclusive"
        )));
    }
    let timeout_patch = payload.timeouts.clone().unwrap_or_default();
    let responses_first_byte_timeout_secs = normalize_optional_timeout_override_secs(
        &timeout_patch.responses_first_byte_timeout_secs,
        "responsesFirstByteTimeoutSecs",
    )
    .map_err(|(_, message)| ApiError::bad_request(anyhow!(message)))?;
    let compact_first_byte_timeout_secs = normalize_optional_timeout_override_secs(
        &timeout_patch.compact_first_byte_timeout_secs,
        "compactFirstByteTimeoutSecs",
    )
    .map_err(|(_, message)| ApiError::bad_request(anyhow!(message)))?;
    let responses_stream_timeout_secs = normalize_optional_timeout_override_secs(
        &timeout_patch.responses_stream_timeout_secs,
        "responsesStreamTimeoutSecs",
    )
    .map_err(|(_, message)| ApiError::bad_request(anyhow!(message)))?;
    let compact_stream_timeout_secs = normalize_optional_timeout_override_secs(
        &timeout_patch.compact_stream_timeout_secs,
        "compactStreamTimeoutSecs",
    )
    .map_err(|(_, message)| ApiError::bad_request(anyhow!(message)))?;
    let existing_row =
        load_prompt_cache_conversation_binding_row(&state.pool, &prompt_cache_key).await?;
    let next_timeout_value = |incoming: Option<Option<i64>>, current: Option<i64>| incoming.unwrap_or(current.map(Some).unwrap_or(None));
    let next_responses_first_byte_timeout_secs =
        next_timeout_value(responses_first_byte_timeout_secs, existing_row.as_ref().and_then(|row| row.responses_first_byte_timeout_secs));
    let next_compact_first_byte_timeout_secs =
        next_timeout_value(compact_first_byte_timeout_secs, existing_row.as_ref().and_then(|row| row.compact_first_byte_timeout_secs));
    let next_responses_stream_timeout_secs =
        next_timeout_value(responses_stream_timeout_secs, existing_row.as_ref().and_then(|row| row.responses_stream_timeout_secs));
    let next_compact_stream_timeout_secs =
        next_timeout_value(compact_stream_timeout_secs, existing_row.as_ref().and_then(|row| row.compact_stream_timeout_secs));
    let next_timeouts_all_clear = next_responses_first_byte_timeout_secs.is_none()
        && next_compact_first_byte_timeout_secs.is_none()
        && next_responses_stream_timeout_secs.is_none()
        && next_compact_stream_timeout_secs.is_none();

    match binding_kind {
        "none" => {
            if next_timeouts_all_clear {
                sqlx::query(
                    "DELETE FROM prompt_cache_conversation_bindings WHERE prompt_cache_key = ?1",
                )
                .bind(&prompt_cache_key)
                .execute(&state.pool)
                .await?;
            } else {
                sqlx::query(
                    r#"
                    INSERT INTO prompt_cache_conversation_bindings (
                        prompt_cache_key,
                        binding_kind,
                        group_name,
                        upstream_account_id,
                        responses_first_byte_timeout_secs,
                        compact_first_byte_timeout_secs,
                        responses_stream_timeout_secs,
                        compact_stream_timeout_secs,
                        created_at,
                        updated_at
                    )
                    VALUES (?1, ?2, NULL, NULL, ?3, ?4, ?5, ?6, datetime('now'), datetime('now'))
                    ON CONFLICT(prompt_cache_key) DO UPDATE SET
                        binding_kind = excluded.binding_kind,
                        group_name = NULL,
                        upstream_account_id = NULL,
                        responses_first_byte_timeout_secs = excluded.responses_first_byte_timeout_secs,
                        compact_first_byte_timeout_secs = excluded.compact_first_byte_timeout_secs,
                        responses_stream_timeout_secs = excluded.responses_stream_timeout_secs,
                        compact_stream_timeout_secs = excluded.compact_stream_timeout_secs,
                        updated_at = excluded.updated_at
                    "#,
                )
                .bind(&prompt_cache_key)
                .bind(PROMPT_CACHE_BINDING_KIND_NONE)
                .bind(next_responses_first_byte_timeout_secs)
                .bind(next_compact_first_byte_timeout_secs)
                .bind(next_responses_stream_timeout_secs)
                .bind(next_compact_stream_timeout_secs)
                .execute(&state.pool)
                .await?;
            }
            let owner =
                load_prompt_cache_encrypted_session_owner_row(&state.pool, &prompt_cache_key)
                    .await?;
            let response = match load_prompt_cache_conversation_binding_row(&state.pool, &prompt_cache_key).await? {
                Some(row) => binding_response_from_row(&state.pool, &state.config, row, owner.as_ref()).await?,
                None => apply_owner_to_none_response(
                    binding_response_for_none(&state.pool, &state.config, prompt_cache_key, owner.as_ref()).await?,
                    owner.as_ref(),
                ),
            };
            Ok(Json(response))
        }
        "group" => {
            let group_name = group_name.ok_or_else(|| {
                ApiError::bad_request(anyhow!("groupName is required for group binding"))
            })?;
            ensure_group_binding_target(&state.pool, &group_name).await?;
            sqlx::query(
                r#"
                INSERT INTO prompt_cache_conversation_bindings (
                    prompt_cache_key,
                    binding_kind,
                    group_name,
                    upstream_account_id,
                    responses_first_byte_timeout_secs,
                    compact_first_byte_timeout_secs,
                    responses_stream_timeout_secs,
                    compact_stream_timeout_secs,
                    created_at,
                    updated_at
                )
                VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, datetime('now'), datetime('now'))
                ON CONFLICT(prompt_cache_key) DO UPDATE SET
                    binding_kind = excluded.binding_kind,
                    group_name = excluded.group_name,
                    upstream_account_id = NULL,
                    responses_first_byte_timeout_secs = excluded.responses_first_byte_timeout_secs,
                    compact_first_byte_timeout_secs = excluded.compact_first_byte_timeout_secs,
                    responses_stream_timeout_secs = excluded.responses_stream_timeout_secs,
                    compact_stream_timeout_secs = excluded.compact_stream_timeout_secs,
                    updated_at = excluded.updated_at
                "#,
            )
            .bind(&prompt_cache_key)
            .bind(PROMPT_CACHE_BINDING_KIND_GROUP)
            .bind(&group_name)
            .bind(next_responses_first_byte_timeout_secs)
            .bind(next_compact_first_byte_timeout_secs)
            .bind(next_responses_stream_timeout_secs)
            .bind(next_compact_stream_timeout_secs)
            .execute(&state.pool)
            .await?;
            let owner =
                load_prompt_cache_encrypted_session_owner_row(&state.pool, &prompt_cache_key)
                    .await?;
            Ok(Json(binding_response_from_row(
                &state.pool,
                &state.config,
                load_prompt_cache_conversation_binding_row(&state.pool, &prompt_cache_key)
                    .await?
                    .expect("saved prompt cache group binding should exist"),
                owner.as_ref(),
            )
            .await?))
        }
        "upstreamAccount" => {
            let upstream_account_id = upstream_account_id.ok_or_else(|| {
                ApiError::bad_request(anyhow!(
                    "upstreamAccountId is required for upstream account binding"
                ))
            })?;
            let _ = ensure_upstream_account_binding_target(&state.pool, upstream_account_id).await?;
            sqlx::query(
                r#"
                INSERT INTO prompt_cache_conversation_bindings (
                    prompt_cache_key,
                    binding_kind,
                    group_name,
                    upstream_account_id,
                    responses_first_byte_timeout_secs,
                    compact_first_byte_timeout_secs,
                    responses_stream_timeout_secs,
                    compact_stream_timeout_secs,
                    created_at,
                    updated_at
                )
                VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, datetime('now'), datetime('now'))
                ON CONFLICT(prompt_cache_key) DO UPDATE SET
                    binding_kind = excluded.binding_kind,
                    group_name = NULL,
                    upstream_account_id = excluded.upstream_account_id,
                    responses_first_byte_timeout_secs = excluded.responses_first_byte_timeout_secs,
                    compact_first_byte_timeout_secs = excluded.compact_first_byte_timeout_secs,
                    responses_stream_timeout_secs = excluded.responses_stream_timeout_secs,
                    compact_stream_timeout_secs = excluded.compact_stream_timeout_secs,
                    updated_at = excluded.updated_at
                "#,
            )
            .bind(&prompt_cache_key)
            .bind(PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT)
            .bind(upstream_account_id)
            .bind(next_responses_first_byte_timeout_secs)
            .bind(next_compact_first_byte_timeout_secs)
            .bind(next_responses_stream_timeout_secs)
            .bind(next_compact_stream_timeout_secs)
            .execute(&state.pool)
            .await?;
            let now_iso = format_utc_iso(Utc::now());
            upsert_sticky_route(&state.pool, &prompt_cache_key, upstream_account_id, &now_iso)
                .await?;
            let owner =
                load_prompt_cache_encrypted_session_owner_row(&state.pool, &prompt_cache_key)
                    .await?;
            Ok(Json(binding_response_from_row(
                &state.pool,
                &state.config,
                load_prompt_cache_conversation_binding_row(&state.pool, &prompt_cache_key)
                    .await?
                    .expect("saved prompt cache account binding should exist"),
                owner.as_ref(),
            )
            .await?))
        }
        _ => Err(ApiError::bad_request(anyhow!(
            "bindingKind must be one of: none, group, upstreamAccount"
        ))),
    }
}
