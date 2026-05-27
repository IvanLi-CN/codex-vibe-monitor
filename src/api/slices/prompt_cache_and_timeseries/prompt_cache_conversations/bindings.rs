use super::*;
const PROMPT_CACHE_BINDING_KIND_GROUP: &str = "group";
const PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT: &str = "upstream_account";

#[derive(Debug, Clone, FromRow)]
pub(crate) struct PromptCacheConversationBindingRow {
    pub(crate) prompt_cache_key: String,
    pub(crate) binding_kind: String,
    pub(crate) group_name: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationBindingResponse {
    pub(crate) prompt_cache_key: String,
    pub(crate) binding_kind: String,
    pub(crate) group_name: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdatePromptCacheConversationBindingRequest {
    binding_kind: String,
    group_name: Option<String>,
    upstream_account_id: Option<i64>,
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

fn binding_response_for_none(prompt_cache_key: String) -> PromptCacheConversationBindingResponse {
    PromptCacheConversationBindingResponse {
        prompt_cache_key,
        binding_kind: "none".to_string(),
        group_name: None,
        upstream_account_id: None,
        upstream_account_name: None,
        updated_at: None,
    }
}

fn binding_response_from_row(row: PromptCacheConversationBindingRow) -> PromptCacheConversationBindingResponse {
    PromptCacheConversationBindingResponse {
        prompt_cache_key: row.prompt_cache_key,
        binding_kind: match row.binding_kind.as_str() {
            PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT => "upstreamAccount".to_string(),
            _ => "group".to_string(),
        },
        group_name: row.group_name,
        upstream_account_id: row.upstream_account_id,
        upstream_account_name: row.upstream_account_name,
        updated_at: Some(row.updated_at),
    }
}

pub(crate) async fn load_prompt_cache_conversation_binding_row(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
) -> Result<Option<PromptCacheConversationBindingRow>> {
    sqlx::query_as::<_, PromptCacheConversationBindingRow>(
        r#"
        SELECT
            binding.prompt_cache_key,
            binding.binding_kind,
            binding.group_name,
            binding.upstream_account_id,
            account.display_name AS upstream_account_name,
            binding.updated_at
        FROM prompt_cache_conversation_bindings AS binding
        LEFT JOIN pool_upstream_accounts AS account
          ON account.id = binding.upstream_account_id
        WHERE binding.prompt_cache_key = ?1
        LIMIT 1
        "#,
    )
    .bind(prompt_cache_key)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
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
    let response = match load_prompt_cache_conversation_binding_row(&state.pool, &prompt_cache_key).await? {
        Some(row) => binding_response_from_row(row),
        None => binding_response_for_none(prompt_cache_key),
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

    match binding_kind {
        "none" => {
            sqlx::query("DELETE FROM prompt_cache_conversation_bindings WHERE prompt_cache_key = ?1")
                .bind(&prompt_cache_key)
                .execute(&state.pool)
                .await?;
            Ok(Json(binding_response_for_none(prompt_cache_key)))
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
                    created_at,
                    updated_at
                )
                VALUES (?1, ?2, ?3, NULL, datetime('now'), datetime('now'))
                ON CONFLICT(prompt_cache_key) DO UPDATE SET
                    binding_kind = excluded.binding_kind,
                    group_name = excluded.group_name,
                    upstream_account_id = NULL,
                    updated_at = excluded.updated_at
                "#,
            )
            .bind(&prompt_cache_key)
            .bind(PROMPT_CACHE_BINDING_KIND_GROUP)
            .bind(&group_name)
            .execute(&state.pool)
            .await?;
            Ok(Json(binding_response_from_row(
                load_prompt_cache_conversation_binding_row(&state.pool, &prompt_cache_key)
                    .await?
                    .expect("saved prompt cache group binding should exist"),
            )))
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
                    created_at,
                    updated_at
                )
                VALUES (?1, ?2, NULL, ?3, datetime('now'), datetime('now'))
                ON CONFLICT(prompt_cache_key) DO UPDATE SET
                    binding_kind = excluded.binding_kind,
                    group_name = NULL,
                    upstream_account_id = excluded.upstream_account_id,
                    updated_at = excluded.updated_at
                "#,
            )
            .bind(&prompt_cache_key)
            .bind(PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT)
            .bind(upstream_account_id)
            .execute(&state.pool)
            .await?;
            let now_iso = format_utc_iso(Utc::now());
            upsert_sticky_route(&state.pool, &prompt_cache_key, upstream_account_id, &now_iso)
                .await?;
            Ok(Json(binding_response_from_row(
                load_prompt_cache_conversation_binding_row(&state.pool, &prompt_cache_key)
                    .await?
                    .expect("saved prompt cache account binding should exist"),
            )))
        }
        _ => Err(ApiError::bad_request(anyhow!(
            "bindingKind must be one of: none, group, upstreamAccount"
        ))),
    }
}
