use super::{
    ApiError, PROMPT_CACHE_BINDING_KIND_GROUP, PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT,
    PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_FORWARD_PROXY,
    PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_REQUEST_REWRITE,
    PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_ROUTING,
    PROMPT_CACHE_CONVERSATION_OPERATION_ORIGIN_SYSTEM_AUTO, PoolRoutingSelectionAudit,
    PromptCacheConversationBindingRow, PromptCacheConversationStickyRouteResponse,
    normalize_sticky_model_key, parse_available_models_json, parse_forward_proxy_keys_json,
};
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Pool, Sqlite};
use std::collections::BTreeMap;
const PROMPT_CACHE_CONVERSATION_OPERATION_EVENTS_DEFAULT_PAGE_SIZE: usize = 20;
const PROMPT_CACHE_CONVERSATION_OPERATION_EVENTS_MAX_PAGE_SIZE: usize = 100;
pub(crate) async fn list_prompt_cache_conversation_operation_events_executor(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
    query: ListPromptCacheConversationOperationEventsQuery,
) -> std::result::Result<PromptCacheConversationOperationEventListResponse, ApiError> {
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
    let total = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM prompt_cache_conversation_operation_events
        WHERE prompt_cache_key = ?1
          AND (
            ?2 IS NULL
            OR EXISTS (
                SELECT 1
                FROM json_each(prompt_cache_conversation_operation_events.info_types_json)
                WHERE json_each.value = ?2
            )
          )
          AND (?3 IS NULL OR json_extract(routing_scope_json, '$.kind') = ?3)
          AND (?4 IS NULL OR json_extract(routing_scope_json, '$.modelKey') = ?4)
        "#,
    )
    .bind(prompt_cache_key)
    .bind(info_type.as_deref())
    .bind(routing_scope.as_deref())
    .bind(routing_model.as_deref())
    .fetch_one(pool)
    .await?;

    let rows = sqlx::query_as::<_, PromptCacheConversationOperationEventRow>(
        r#"
        SELECT
            id,
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
            routing_scope_json,
            sticky_transitions_json
        FROM prompt_cache_conversation_operation_events
        WHERE prompt_cache_key = ?1
          AND (
            ?2 IS NULL
            OR EXISTS (
                SELECT 1
                FROM json_each(prompt_cache_conversation_operation_events.info_types_json)
                WHERE json_each.value = ?2
            )
          )
          AND (?3 IS NULL OR json_extract(routing_scope_json, '$.kind') = ?3)
          AND (?4 IS NULL OR json_extract(routing_scope_json, '$.modelKey') = ?4)
        ORDER BY occurred_at DESC, id DESC
        LIMIT ?5 OFFSET ?6
        "#,
    )
    .bind(prompt_cache_key)
    .bind(info_type.as_deref())
    .bind(routing_scope.as_deref())
    .bind(routing_model.as_deref())
    .bind(page_size as i64)
    .bind(offset as i64)
    .fetch_all(pool)
    .await?;

    let routing_model_facets = sqlx::query_scalar::<_, String>(
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
    .fetch_all(pool)
    .await?;

    Ok(PromptCacheConversationOperationEventListResponse {
        items: rows
            .into_iter()
            .map(prompt_cache_conversation_operation_event_response_from_row)
            .collect(),
        total,
        page,
        page_size,
        routing_model_facets,
    })
}
#[derive(Debug, Clone, FromRow)]
struct PromptCacheConversationOperationEventRow {
    pub(crate) id: i64,
    pub(crate) prompt_cache_key: String,
    pub(crate) action: String,
    pub(crate) origin: String,
    pub(crate) info_types_json: String,
    pub(crate) occurred_at: String,
    pub(crate) headline: String,
    pub(crate) changed_fields_json: Option<String>,
    pub(crate) binding_before_json: Option<String>,
    pub(crate) binding_after_json: Option<String>,
    pub(crate) sticky_before_json: Option<String>,
    pub(crate) sticky_after_json: Option<String>,
    pub(crate) invoke_id: Option<String>,
    pub(crate) routing_context_json: Option<String>,
    pub(crate) routing_scope_json: Option<String>,
    pub(crate) sticky_transitions_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationOperationBindingSnapshot {
    pub(crate) binding_kind: String,
    pub(crate) group_name: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationOperationStickySnapshot {
    pub(crate) upstream_account_id: i64,
    pub(crate) upstream_account_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationOperationRoutingContext {
    pub(crate) reason_code: String,
    pub(crate) routing_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) routing_selection_audit: Option<PoolRoutingSelectionAudit>,
    pub(crate) http_status: Option<u16>,
    pub(crate) trigger_attempt_id: Option<String>,
    pub(crate) causing_attempt_id: Option<String>,
    pub(crate) causing_http_status: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationOperationRoutingScope {
    pub(crate) kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) model_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) request_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationOperationStickyTransition {
    pub(crate) model_key: Option<String>,
    pub(crate) before: Option<PromptCacheConversationOperationStickySnapshot>,
    pub(crate) after: Option<PromptCacheConversationOperationStickySnapshot>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationOperationEventResponse {
    pub(crate) id: i64,
    pub(crate) prompt_cache_key: String,
    pub(crate) action: String,
    pub(crate) origin: String,
    pub(crate) info_types: Vec<String>,
    pub(crate) occurred_at: String,
    pub(crate) headline: String,
    pub(crate) changed_fields: Vec<String>,
    pub(crate) binding_before: Option<PromptCacheConversationOperationBindingSnapshot>,
    pub(crate) binding_after: Option<PromptCacheConversationOperationBindingSnapshot>,
    pub(crate) sticky_before: Option<PromptCacheConversationOperationStickySnapshot>,
    pub(crate) sticky_after: Option<PromptCacheConversationOperationStickySnapshot>,
    pub(crate) invoke_id: Option<String>,
    pub(crate) routing_context: Option<PromptCacheConversationOperationRoutingContext>,
    pub(crate) routing_scope: Option<PromptCacheConversationOperationRoutingScope>,
    pub(crate) sticky_transitions: Vec<PromptCacheConversationOperationStickyTransition>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationOperationEventListResponse {
    pub(crate) items: Vec<PromptCacheConversationOperationEventResponse>,
    pub(crate) total: i64,
    pub(crate) page: usize,
    pub(crate) page_size: usize,
    pub(crate) routing_model_facets: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListPromptCacheConversationOperationEventsQuery {
    pub(crate) page: Option<usize>,
    pub(crate) page_size: Option<usize>,
    pub(crate) info_type: Option<String>,
    pub(crate) routing_scope: Option<String>,
    pub(crate) routing_model: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct AppendPromptCacheConversationOperationEventInput {
    pub(super) prompt_cache_key: String,
    pub(super) action: String,
    pub(super) origin: String,
    pub(super) info_types: Vec<String>,
    pub(super) occurred_at: String,
    pub(super) headline: String,
    pub(super) changed_fields: Vec<String>,
    pub(super) binding_before: Option<PromptCacheConversationOperationBindingSnapshot>,
    pub(super) binding_after: Option<PromptCacheConversationOperationBindingSnapshot>,
    pub(super) sticky_before: Option<PromptCacheConversationOperationStickySnapshot>,
    pub(super) sticky_after: Option<PromptCacheConversationOperationStickySnapshot>,
    pub(super) invoke_id: Option<String>,
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

pub(crate) async fn load_prompt_cache_conversation_sticky_routes(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
) -> Result<Vec<PromptCacheConversationStickyRouteResponse>> {
    load_prompt_cache_conversation_sticky_routes_executor(pool, prompt_cache_key).await
}

pub(crate) async fn load_prompt_cache_conversation_sticky_routes_executor<'e, E>(
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

pub(crate) async fn load_prompt_cache_conversation_sticky_snapshot_for_model_executor<'e, E>(
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

pub(crate) fn prompt_cache_conversation_operation_binding_snapshot_from_row(
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

pub(crate) fn prompt_cache_conversation_bound_proxy_keys_from_row(
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

pub(crate) fn prompt_cache_conversation_policy_changed_fields(
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

pub(crate) fn prompt_cache_conversation_policy_info_types(
    changed_fields: &[String],
) -> Vec<String> {
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

pub(crate) fn prompt_cache_conversation_operation_headline(action: &str) -> String {
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

pub(crate) fn parse_prompt_cache_conversation_operation_string_array_json(
    value: Option<&str>,
) -> Vec<String> {
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

pub(crate) fn normalize_prompt_cache_conversation_operation_info_type(
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

pub(crate) fn normalize_prompt_cache_conversation_operation_routing_scope(
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

pub(crate) fn normalize_prompt_cache_conversation_operation_routing_model(
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

#[derive(Debug)]
struct PromptCacheConversationOperationEventPayload {
    input: AppendPromptCacheConversationOperationEventInput,
    routing_context: Option<PromptCacheConversationOperationRoutingContext>,
    routing_scope: Option<PromptCacheConversationOperationRoutingScope>,
    sticky_transitions: Option<Vec<PromptCacheConversationOperationStickyTransition>>,
}

impl PromptCacheConversationOperationEventPayload {
    fn with_routing_context(
        input: AppendPromptCacheConversationOperationEventInput,
        routing_context: Option<PromptCacheConversationOperationRoutingContext>,
    ) -> Self {
        let routing_scope = input
            .info_types
            .iter()
            .any(|value| value == PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_ROUTING)
            .then(|| PromptCacheConversationOperationRoutingScope {
                kind: "all".to_string(),
                model_key: None,
                request_model: None,
            });
        Self {
            input,
            routing_context,
            routing_scope,
            sticky_transitions: None,
        }
    }

    fn with_routing_scope(
        input: AppendPromptCacheConversationOperationEventInput,
        routing_context: Option<PromptCacheConversationOperationRoutingContext>,
        routing_scope: PromptCacheConversationOperationRoutingScope,
    ) -> Self {
        Self {
            input,
            routing_context,
            routing_scope: Some(routing_scope),
            sticky_transitions: None,
        }
    }

    fn with_sticky_transitions(
        input: AppendPromptCacheConversationOperationEventInput,
        sticky_transitions: &[PromptCacheConversationOperationStickyTransition],
    ) -> Self {
        Self {
            input,
            routing_context: None,
            routing_scope: Some(PromptCacheConversationOperationRoutingScope {
                kind: "all".to_string(),
                model_key: None,
                request_model: None,
            }),
            sticky_transitions: Some(sticky_transitions.to_vec()),
        }
    }
}

async fn write_prompt_cache_conversation_operation_event_executor<'e, E>(
    executor: E,
    payload: PromptCacheConversationOperationEventPayload,
) -> Result<()>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let PromptCacheConversationOperationEventPayload {
        input:
            AppendPromptCacheConversationOperationEventInput {
                prompt_cache_key,
                action,
                origin,
                info_types,
                occurred_at,
                headline,
                changed_fields,
                binding_before,
                binding_after,
                sticky_before,
                sticky_after,
                invoke_id,
            },
        routing_context,
        routing_scope,
        sticky_transitions,
    } = payload;
    let changed_fields_json = (!changed_fields.is_empty())
        .then(|| serde_json::to_string(&changed_fields))
        .transpose()?;
    let binding_before_json = binding_before
        .map(|value| serde_json::to_string(&value))
        .transpose()?;
    let binding_after_json = binding_after
        .map(|value| serde_json::to_string(&value))
        .transpose()?;
    let sticky_before_json = sticky_before
        .map(|value| serde_json::to_string(&value))
        .transpose()?;
    let sticky_after_json = sticky_after
        .map(|value| serde_json::to_string(&value))
        .transpose()?;
    let routing_context_json = routing_context
        .map(|value| serde_json::to_string(&value))
        .transpose()?;
    let routing_scope_json = routing_scope
        .map(|value| serde_json::to_string(&value))
        .transpose()?;
    let sticky_transitions_json = sticky_transitions
        .map(|value| serde_json::to_string(&value))
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
            routing_scope_json,
            sticky_transitions_json
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
        "#,
    )
    .bind(prompt_cache_key)
    .bind(action)
    .bind(origin)
    .bind(serde_json::to_string(&info_types)?)
    .bind(occurred_at)
    .bind(headline)
    .bind(changed_fields_json)
    .bind(binding_before_json)
    .bind(binding_after_json)
    .bind(sticky_before_json)
    .bind(sticky_after_json)
    .bind(invoke_id)
    .bind(routing_context_json)
    .bind(routing_scope_json)
    .bind(sticky_transitions_json)
    .execute(executor)
    .await?;
    Ok(())
}

pub(super) async fn append_prompt_cache_conversation_operation_event_executor<'e, E>(
    executor: E,
    input: AppendPromptCacheConversationOperationEventInput,
) -> Result<()>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    write_prompt_cache_conversation_operation_event_executor(
        executor,
        PromptCacheConversationOperationEventPayload::with_routing_context(input, None),
    )
    .await
}

pub(super) async fn append_prompt_cache_conversation_operation_event_with_routing_context_executor<
    'e,
    E,
>(
    executor: E,
    input: AppendPromptCacheConversationOperationEventInput,
    routing_context: Option<PromptCacheConversationOperationRoutingContext>,
) -> Result<()>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    write_prompt_cache_conversation_operation_event_executor(
        executor,
        PromptCacheConversationOperationEventPayload::with_routing_context(input, routing_context),
    )
    .await
}

pub(super) async fn append_prompt_cache_conversation_operation_event_with_routing_scope_executor<
    'e,
    E,
>(
    executor: E,
    input: AppendPromptCacheConversationOperationEventInput,
    routing_context: Option<PromptCacheConversationOperationRoutingContext>,
    routing_scope: PromptCacheConversationOperationRoutingScope,
) -> Result<()>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    write_prompt_cache_conversation_operation_event_executor(
        executor,
        PromptCacheConversationOperationEventPayload::with_routing_scope(
            input,
            routing_context,
            routing_scope,
        ),
    )
    .await
}

pub(super) async fn append_prompt_cache_conversation_operation_event_with_sticky_transitions_executor<
    'e,
    E,
>(
    executor: E,
    input: AppendPromptCacheConversationOperationEventInput,
    sticky_transitions: &[PromptCacheConversationOperationStickyTransition],
) -> Result<()>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    write_prompt_cache_conversation_operation_event_executor(
        executor,
        PromptCacheConversationOperationEventPayload::with_sticky_transitions(
            input,
            sticky_transitions,
        ),
    )
    .await
}

pub(crate) fn prompt_cache_conversation_sticky_transitions(
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

pub(crate) async fn append_runtime_sticky_target_cleared_event_executor<'e, E>(
    executor: E,
    prompt_cache_key: &str,
    account_id: i64,
    account_name: Option<String>,
    occurred_at: &str,
    invoke_id: Option<String>,
    routing_context: PromptCacheConversationOperationRoutingContext,
    routing_scope: PromptCacheConversationOperationRoutingScope,
) -> Result<()>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let sticky_before = PromptCacheConversationOperationStickySnapshot {
        upstream_account_id: account_id,
        upstream_account_name: account_name,
    };
    write_prompt_cache_conversation_operation_event_executor(
        executor,
        PromptCacheConversationOperationEventPayload::with_routing_scope(
            AppendPromptCacheConversationOperationEventInput {
                prompt_cache_key: prompt_cache_key.to_string(),
                action: "stickyTargetCleared".to_string(),
                origin: PROMPT_CACHE_CONVERSATION_OPERATION_ORIGIN_SYSTEM_AUTO.to_string(),
                info_types: vec![PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_ROUTING.to_string()],
                occurred_at: occurred_at.to_string(),
                headline: prompt_cache_conversation_operation_headline("stickyTargetCleared"),
                changed_fields: vec!["stickyTarget".to_string()],
                binding_before: None,
                binding_after: None,
                sticky_before: Some(sticky_before),
                sticky_after: None,
                invoke_id,
            },
            Some(routing_context),
            routing_scope,
        ),
    )
    .await
}

pub(crate) async fn load_runtime_attempt_routing_context_executor<'e, E>(
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

pub(crate) async fn load_pending_sticky_clear_cause_executor<'e, E>(
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

pub(super) async fn append_prompt_cache_conversation_operation_event(
    pool: &Pool<Sqlite>,
    input: AppendPromptCacheConversationOperationEventInput,
) -> Result<()> {
    append_prompt_cache_conversation_operation_event_executor(pool, input).await
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
