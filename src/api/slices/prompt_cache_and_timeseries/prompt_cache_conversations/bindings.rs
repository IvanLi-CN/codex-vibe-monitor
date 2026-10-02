use super::*;
use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::collections::HashSet;
pub(crate) const PROMPT_CACHE_BINDING_KIND_GROUP: &str = "group";
pub(crate) const PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT: &str = "upstream_account";
pub(crate) const PROMPT_CACHE_BINDING_KIND_NONE: &str = "none";
pub(crate) const PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_ROUTING: &str = "routing";
pub(crate) const PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_FORWARD_PROXY: &str = "forwardProxy";
pub(crate) const PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_REQUEST_REWRITE: &str =
    "requestRewrite";
pub(crate) const PROMPT_CACHE_CONVERSATION_OPERATION_ORIGIN_DETAIL_DRAWER: &str = "detailDrawer";
pub(crate) const PROMPT_CACHE_CONVERSATION_OPERATION_ORIGIN_DASHBOARD_BULK: &str = "dashboardBulk";
pub(crate) const PROMPT_CACHE_CONVERSATION_OPERATION_ORIGIN_SYSTEM_AUTO: &str = "systemAuto";
static PROMPT_CACHE_BINDING_WRITE_LOCK: once_cell::sync::Lazy<tokio::sync::Mutex<()>> =
    once_cell::sync::Lazy::new(|| tokio::sync::Mutex::new(()));

#[path = "bindings/operation_log.rs"]
mod operation_log;
#[path = "bindings/routing_policy.rs"]
mod routing_policy;

use operation_log::{
    AppendPromptCacheConversationOperationEventInput,
    append_prompt_cache_conversation_operation_event,
    append_prompt_cache_conversation_operation_event_with_routing_scope_executor,
    append_prompt_cache_conversation_operation_event_with_sticky_transitions_executor,
    load_pending_sticky_clear_cause_executor, load_prompt_cache_conversation_sticky_routes,
    load_prompt_cache_conversation_sticky_routes_executor,
    load_prompt_cache_conversation_sticky_snapshot,
    load_prompt_cache_conversation_sticky_snapshot_executor,
    load_prompt_cache_conversation_sticky_snapshot_for_model_executor,
    load_runtime_attempt_routing_context_executor,
    prompt_cache_conversation_operation_binding_snapshot_from_row,
    prompt_cache_conversation_operation_headline, prompt_cache_conversation_policy_changed_fields,
    prompt_cache_conversation_policy_info_types, prompt_cache_conversation_sticky_transitions,
};
pub(crate) use operation_log::{
    ListPromptCacheConversationOperationEventsQuery,
    PromptCacheConversationOperationEventListResponse,
    PromptCacheConversationOperationRoutingContext, PromptCacheConversationOperationRoutingScope,
    append_runtime_sticky_target_cleared_event_executor, load_sticky_account_id,
};
pub(crate) use routing_policy::{
    PromptCacheConversationBindingConstraint, PromptCacheEncryptedSessionOwnerRow,
    binding_constraint_accepts_upstream_account_id,
    confirm_prompt_cache_encrypted_session_owner_success, ensure_group_binding_target,
    ensure_upstream_account_binding_target, load_prompt_cache_conversation_routing_override,
    load_prompt_cache_encrypted_session_owner_row,
    promote_prompt_cache_group_binding_to_upstream_account_and_broadcast,
    resolve_prompt_cache_effective_routing_constraint,
};
use routing_policy::{
    delete_prompt_cache_encrypted_session_owner_executor,
    load_prompt_cache_encrypted_session_owner_row_if_enabled,
};
#[cfg(test)]
pub(crate) use routing_policy::{
    promote_prompt_cache_group_binding_to_upstream_account,
    upsert_prompt_cache_encrypted_session_owner,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeStickyMutation {
    Unchanged,
    Changed {
        previous_upstream_account_id: Option<i64>,
    },
    Suppressed,
}

impl RuntimeStickyMutation {
    pub(crate) fn writes_conversation_operation(self) -> bool {
        matches!(self, Self::Changed { .. } | Self::Suppressed)
    }

    pub(crate) fn previous_upstream_account_id(self) -> Option<i64> {
        match self {
            Self::Changed {
                previous_upstream_account_id,
            } => previous_upstream_account_id,
            Self::Unchanged | Self::Suppressed => None,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct PromptCacheConversationBindingRow {
    pub(crate) prompt_cache_key: String,
    pub(crate) binding_kind: String,
    pub(crate) group_name: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) responses_first_byte_timeout_secs: Option<i64>,
    pub(crate) compact_first_byte_timeout_secs: Option<i64>,
    pub(crate) image_first_byte_timeout_secs: Option<i64>,
    pub(crate) responses_stream_timeout_secs: Option<i64>,
    pub(crate) compact_stream_timeout_secs: Option<i64>,
    pub(crate) allow_switch_upstream: Option<i64>,
    pub(crate) fast_mode_rewrite_mode: Option<String>,
    pub(crate) image_tool_rewrite_mode: Option<String>,
    pub(crate) codex_imagegen_rewrite_mode: Option<String>,
    pub(crate) available_models_json: Option<String>,
    #[sqlx(default)]
    pub(crate) available_models_mode: Option<String>,
    pub(crate) forward_proxy_key: Option<String>,
    pub(crate) forward_proxy_keys_json: Option<String>,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationPolicyFieldSources {
    pub(crate) allow_switch_upstream: String,
    pub(crate) fast_mode_rewrite_mode: String,
    pub(crate) image_tool_rewrite_mode: String,
    pub(crate) codex_imagegen_rewrite_mode: String,
    pub(crate) available_models: String,
    pub(crate) available_models_mode: String,
    pub(crate) forward_proxy_key: String,
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
    pub(crate) sticky_routes: Vec<PromptCacheConversationStickyRouteResponse>,
    pub(crate) timeouts: RoutingTimeoutSettings,
    pub(crate) timeout_field_sources: RoutingTimeoutFieldSources,
    pub(crate) allow_switch_upstream: Option<bool>,
    pub(crate) fast_mode_rewrite_mode: Option<TagFastModeRewriteMode>,
    pub(crate) image_tool_rewrite_mode: Option<ImageToolRewriteMode>,
    pub(crate) codex_imagegen_rewrite_mode: Option<CodexImagegenRewriteMode>,
    pub(crate) available_models: Option<Vec<String>>,
    pub(crate) available_models_mode: Option<AvailableModelsMode>,
    pub(crate) forward_proxy_key: Option<String>,
    pub(crate) forward_proxy_keys: Vec<String>,
    pub(crate) policy_field_sources: PromptCacheConversationPolicyFieldSources,
    pub(crate) updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationStickyRouteResponse {
    pub(crate) model_key: Option<String>,
    pub(crate) upstream_account_id: i64,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) last_seen_at: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) enum PatchField<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}

impl<T> PatchField<T> {
    fn map<U>(self, f: impl FnOnce(T) -> U) -> PatchField<U> {
        match self {
            Self::Missing => PatchField::Missing,
            Self::Null => PatchField::Null,
            Self::Value(value) => PatchField::Value(f(value)),
        }
    }
}

pub(crate) fn deserialize_patch_field<'de, D, T>(deserializer: D) -> Result<PatchField<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let raw = serde_json::Value::deserialize(deserializer)?;
    if raw.is_null() {
        return Ok(PatchField::Null);
    }
    serde_json::from_value(raw)
        .map(PatchField::Value)
        .map_err(serde::de::Error::custom)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdatePromptCacheConversationBindingRequest {
    binding_kind: String,
    group_name: Option<String>,
    upstream_account_id: Option<i64>,
    #[serde(default)]
    timeouts: Option<UpdateRoutingTimeoutSettingsRequest>,
    #[serde(default, deserialize_with = "deserialize_patch_field")]
    allow_switch_upstream: PatchField<bool>,
    #[serde(default, deserialize_with = "deserialize_patch_field")]
    fast_mode_rewrite_mode: PatchField<String>,
    #[serde(default, deserialize_with = "deserialize_patch_field")]
    image_tool_rewrite_mode: PatchField<String>,
    #[serde(default, deserialize_with = "deserialize_patch_field")]
    codex_imagegen_rewrite_mode: PatchField<String>,
    #[serde(default, deserialize_with = "deserialize_patch_field")]
    available_models: PatchField<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_patch_field")]
    available_models_mode: PatchField<String>,
    #[serde(default, deserialize_with = "deserialize_patch_field")]
    forward_proxy_key: PatchField<String>,
    #[serde(default, deserialize_with = "deserialize_patch_field")]
    forward_proxy_keys: PatchField<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkPromptCacheConversationBindingsRequest {
    prompt_cache_keys: Vec<String>,
    #[serde(flatten)]
    action: BulkPromptCacheConversationBindingsAction,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
pub(crate) enum BulkPromptCacheConversationBindingsAction {
    Bind {
        #[serde(rename = "bindingKind")]
        binding_kind: String,
        #[serde(rename = "groupName")]
        group_name: Option<String>,
        #[serde(rename = "upstreamAccountId")]
        upstream_account_id: Option<i64>,
    },
    ClearAndResetAffinity,
    SetFastModeRewriteMode {
        #[serde(rename = "fastModeRewriteMode")]
        fast_mode_rewrite_mode: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkPromptCacheConversationBindingItemResponse {
    pub(crate) prompt_cache_key: String,
    pub(crate) ok: bool,
    pub(crate) error: Option<String>,
    pub(crate) binding: Option<PromptCacheConversationBindingResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkPromptCacheConversationBindingsResponse {
    pub(crate) action: String,
    pub(crate) total_requested: usize,
    pub(crate) total_succeeded: usize,
    pub(crate) total_failed: usize,
    pub(crate) items: Vec<BulkPromptCacheConversationBindingItemResponse>,
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
    let effective_policy = if row.binding_kind == PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT {
        if let Some(account_id) = row.upstream_account_id {
            Some(load_effective_routing_rule_for_account(&state.pool, account_id).await?)
        } else {
            None
        }
    } else if row.binding_kind == PROMPT_CACHE_BINDING_KIND_GROUP {
        Some(load_effective_routing_rule_for_group(&state.pool, row.group_name.as_deref()).await?)
    } else if row.binding_kind == PROMPT_CACHE_BINDING_KIND_NONE {
        if let Some(owner) = owner {
            Some(
                load_effective_routing_rule_for_account(
                    &state.pool,
                    owner.owner_upstream_account_id,
                )
                .await?,
            )
        } else if let Some(account_id) = sticky_account_id {
            Some(load_effective_routing_rule_for_account(&state.pool, account_id).await?)
        } else {
            Some(load_effective_routing_rule_for_group(&state.pool, None).await?)
        }
    } else {
        None
    };
    let (legacy_available_models, available_models_invalid) = row
        .available_models_json
        .as_deref()
        .map(parse_available_models_json_with_invalid)
        .unwrap_or((None, false));
    let policy_field_sources =
        PromptCacheConversationPolicyFieldSources::from_row(&row, effective_policy.as_ref());
    let effective_available_models = effective_policy
        .as_ref()
        .and_then(|rule| rule.available_models().map(|models| models.to_vec()));
    let (timeouts, timeout_field_sources) =
        if row.binding_kind == PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT {
            if let Some(account_id) = row.upstream_account_id {
                let (timeouts, sources, _) = load_effective_request_path_timeouts_for_account(
                    &state.pool,
                    config,
                    account_id,
                    Some(row.prompt_cache_key.as_str()),
                )
                .await?;
                (timeouts, sources)
            } else {
                let (timeouts, sources, _) =
                    load_effective_request_path_timeouts_for_group_and_conversation(
                        &state.pool,
                        config,
                        None,
                        Some(row.prompt_cache_key.as_str()),
                    )
                    .await?;
                (timeouts, sources)
            }
        } else if row.binding_kind == PROMPT_CACHE_BINDING_KIND_GROUP {
            let (timeouts, sources, _) =
                load_effective_request_path_timeouts_for_group_and_conversation(
                    &state.pool,
                    config,
                    row.group_name.as_deref(),
                    Some(row.prompt_cache_key.as_str()),
                )
                .await?;
            (timeouts, sources)
        } else if let Some(owner) = owner {
            let (timeouts, sources, _) = load_effective_request_path_timeouts_for_account(
                &state.pool,
                config,
                owner.owner_upstream_account_id,
                Some(row.prompt_cache_key.as_str()),
            )
            .await?;
            (timeouts, sources)
        } else {
            let (timeouts, sources, _) =
                load_effective_request_path_timeouts_for_group_and_conversation(
                    &state.pool,
                    config,
                    None,
                    Some(row.prompt_cache_key.as_str()),
                )
                .await?;
            (timeouts, sources)
        };
    let row_forward_proxy_keys =
        parse_forward_proxy_keys_json(row.forward_proxy_keys_json.as_deref());
    let forward_proxy_key = row
        .forward_proxy_key
        .clone()
        .or_else(|| row_forward_proxy_keys.first().cloned())
        .or(
            resolve_effective_forward_proxy_key_for_row(state, &row, owner, sticky_account_id)
                .await,
        );
    let forward_proxy_keys = if row_forward_proxy_keys.is_empty() {
        forward_proxy_key.iter().cloned().collect::<Vec<_>>()
    } else {
        row_forward_proxy_keys
    };
    let sticky_routes =
        load_prompt_cache_conversation_sticky_routes(&state.pool, &row.prompt_cache_key).await?;

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
    })
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

pub(crate) async fn upsert_runtime_prompt_cache_conversation_sticky_route(
    pool: &Pool<Sqlite>,
    sticky_key: &str,
    prompt_cache_key: Option<&str>,
    upstream_account_id: i64,
    now_iso: &str,
    invoke_id: Option<&str>,
    attempt_id: Option<i64>,
    sticky_affinity_generation: Option<i64>,
) -> Result<RuntimeStickyMutation> {
    let event_prompt_cache_key = prompt_cache_key.filter(|key| *key == sticky_key);
    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(conn.as_mut())
        .await
        .context("failed to acquire runtime sticky event write lock")?;

    let write_outcome: Result<RuntimeStickyMutation> = async {
        let (
            trigger_attempt_id,
            routing_source,
            routing_selection_audit,
            trigger_invoke_id,
            request_model,
        ) = load_runtime_attempt_routing_context_executor(conn.as_mut(), attempt_id).await?;
        let current_epoch =
            load_sticky_affinity_generation_executor(conn.as_mut(), sticky_key).await?;
        let model_key = normalize_sticky_model_key(request_model.as_deref());
        let pending_clear_cause = load_pending_sticky_clear_cause_executor(
            conn.as_mut(),
            sticky_key,
            model_key.as_deref(),
        )
        .await?;
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
        let current_generation = if let Some(model_key) = model_key.as_deref() {
            load_sticky_model_generation_executor(conn.as_mut(), sticky_key, model_key).await?
        } else {
            current_epoch
        };
        let sticky_before = if event_prompt_cache_key.is_some() {
            load_prompt_cache_conversation_sticky_snapshot_for_model_executor(
                conn.as_mut(),
                sticky_key,
                model_key.as_deref(),
            )
            .await?
        } else {
            None
        };
        if let Some(expected_generation) = sticky_affinity_generation
            && (if model_key.is_some() {
                let (expected_epoch, expected_model_generation) =
                    unpack_sticky_affinity_token(expected_generation);
                current_epoch != expected_epoch || current_generation != expected_model_generation
            } else {
                current_generation != expected_generation
            })
        {
            if let Some(prompt_cache_key) = event_prompt_cache_key {
                append_prompt_cache_conversation_operation_event_with_routing_scope_executor(
                    conn.as_mut(),
                    AppendPromptCacheConversationOperationEventInput {
                        prompt_cache_key: prompt_cache_key.to_string(),
                        action: "stickyMutationSuppressed".to_string(),
                        origin: PROMPT_CACHE_CONVERSATION_OPERATION_ORIGIN_SYSTEM_AUTO.to_string(),
                        info_types: vec![
                            PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_ROUTING.to_string(),
                        ],
                        occurred_at: now_iso.to_string(),
                        headline: prompt_cache_conversation_operation_headline(
                            "stickyMutationSuppressed",
                        ),
                        changed_fields: Vec::new(),
                        binding_before: None,
                        binding_after: None,
                        sticky_before: sticky_before.clone(),
                        sticky_after: sticky_before,
                        invoke_id: invoke_id
                            .map(ToOwned::to_owned)
                            .or_else(|| trigger_invoke_id.clone()),
                    },
                    Some(PromptCacheConversationOperationRoutingContext {
                        reason_code: "staleConcurrentCompletion".to_string(),
                        routing_source,
                        routing_selection_audit: routing_selection_audit.clone(),
                        http_status: None,
                        trigger_attempt_id,
                        causing_attempt_id: None,
                        causing_http_status: None,
                    }),
                    routing_scope.clone(),
                )
                .await?;
            }
            return Ok(RuntimeStickyMutation::Suppressed);
        }

        let previous_account_id = if let Some(model_key) = model_key.as_deref() {
            sqlx::query_scalar::<_, i64>(
                "SELECT account_id FROM pool_sticky_model_routes WHERE sticky_key = ?1 AND model_key = ?2 LIMIT 1",
            )
            .bind(sticky_key)
            .bind(model_key)
            .fetch_optional(conn.as_mut())
            .await?
        } else {
            sqlx::query_scalar::<_, i64>(
                "SELECT account_id FROM pool_sticky_routes WHERE sticky_key = ?1 LIMIT 1",
            )
            .bind(sticky_key)
            .fetch_optional(conn.as_mut())
            .await?
        };
        let target_changed = previous_account_id != Some(upstream_account_id);
        if let Some(model_key) = model_key.as_deref() {
            upsert_sticky_model_route_executor(
                conn.as_mut(),
                sticky_key,
                model_key,
                upstream_account_id,
                now_iso,
            )
            .await?;
            if target_changed {
                bump_sticky_model_generation_executor(conn.as_mut(), sticky_key, model_key, now_iso)
                    .await?;
            }
        } else {
            upsert_sticky_route_executor(conn.as_mut(), sticky_key, upstream_account_id, now_iso)
                .await?;
            if target_changed {
                bump_sticky_affinity_generation_executor(conn.as_mut(), sticky_key, now_iso).await?;
            }
        }

        if let Some(prompt_cache_key) = event_prompt_cache_key {
            let sticky_after = load_prompt_cache_conversation_sticky_snapshot_for_model_executor(
                conn.as_mut(),
                sticky_key,
                model_key.as_deref(),
            )
            .await?;

            let sticky_changed = sticky_before != sticky_after;
            if sticky_changed && let Some(sticky_after) = sticky_after {
                let (causing_attempt_id, causing_http_status) = if previous_account_id.is_none() {
                    pending_clear_cause
                } else {
                    (None, None)
                };
                append_prompt_cache_conversation_operation_event_with_routing_scope_executor(
                    conn.as_mut(),
                    AppendPromptCacheConversationOperationEventInput {
                        prompt_cache_key: prompt_cache_key.to_string(),
                        action: "stickyTargetChanged".to_string(),
                        origin: PROMPT_CACHE_CONVERSATION_OPERATION_ORIGIN_SYSTEM_AUTO.to_string(),
                        info_types: vec![
                            PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_ROUTING.to_string(),
                        ],
                        occurred_at: now_iso.to_string(),
                        headline: prompt_cache_conversation_operation_headline(
                            "stickyTargetChanged",
                        ),
                        changed_fields: vec!["stickyTarget".to_string()],
                        binding_before: None,
                        binding_after: None,
                        sticky_before,
                        sticky_after: Some(sticky_after),
                        invoke_id: invoke_id
                            .map(ToOwned::to_owned)
                            .or_else(|| trigger_invoke_id.clone()),
                    },
                    Some(PromptCacheConversationOperationRoutingContext {
                        reason_code: if causing_attempt_id.is_some() {
                            "freshAssignmentAfterFailure".to_string()
                        } else if previous_account_id.is_some() {
                            "stickyTargetChanged".to_string()
                        } else {
                            "firstSuccessfulAssignment".to_string()
                        },
                        routing_source,
                        routing_selection_audit,
                        http_status: None,
                        trigger_attempt_id,
                        causing_attempt_id,
                        causing_http_status,
                    }),
                    routing_scope,
                )
                .await?;
            }
        }
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

pub(crate) async fn list_prompt_cache_conversation_operation_events(
    State(state): State<Arc<AppState>>,
    AxumPath(encoded_prompt_cache_key): AxumPath<String>,
    Query(query): Query<ListPromptCacheConversationOperationEventsQuery>,
) -> Result<Json<PromptCacheConversationOperationEventListResponse>, ApiError> {
    let prompt_cache_key = normalize_prompt_cache_conversation_key(&encoded_prompt_cache_key)?;
    Ok(Json(
        operation_log::list_prompt_cache_conversation_operation_events_executor(
            &state.pool,
            &prompt_cache_key,
            query,
        )
        .await?,
    ))
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

async fn save_prompt_cache_conversation_binding_for_key(
    state: &AppState,
    prompt_cache_key: &str,
    payload: UpdatePromptCacheConversationBindingRequest,
    origin: &str,
) -> Result<PromptCacheConversationBindingResponse, ApiError> {
    let _write_guard = PROMPT_CACHE_BINDING_WRITE_LOCK.lock().await;
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
    let image_first_byte_timeout_secs = normalize_optional_timeout_override_secs(
        &timeout_patch.image_first_byte_timeout_secs,
        "imageFirstByteTimeoutSecs",
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
    let allow_switch_upstream = payload
        .allow_switch_upstream
        .map(|enabled| if enabled { 1 } else { 0 });
    let fast_mode_rewrite_mode = normalize_fast_mode_rewrite_mode(payload.fast_mode_rewrite_mode)?
        .map(|mode| mode.as_str().to_string());
    let image_tool_rewrite_mode =
        normalize_image_tool_rewrite_mode(payload.image_tool_rewrite_mode)?
            .map(|mode| mode.as_str().to_string());
    let codex_imagegen_rewrite_mode =
        normalize_codex_imagegen_rewrite_mode(payload.codex_imagegen_rewrite_mode)?
            .map(|mode| mode.as_str().to_string());
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
        normalize_forward_proxy_keys_patch(state, payload.forward_proxy_keys).await?;
    let forward_proxy_keys = match forward_proxy_keys {
        PatchField::Missing => match &forward_proxy_key {
            PatchField::Missing => PatchField::Missing,
            PatchField::Null => PatchField::Null,
            PatchField::Value(proxy_key) => PatchField::Value(vec![proxy_key.clone()]),
        },
        value => value,
    };
    let existing_row =
        load_prompt_cache_conversation_binding_row(&state.pool, prompt_cache_key).await?;
    let sticky_before =
        load_prompt_cache_conversation_sticky_snapshot(&state.pool, prompt_cache_key).await?;
    let sticky_routes_before =
        load_prompt_cache_conversation_sticky_routes(&state.pool, prompt_cache_key).await?;
    let next_responses_first_byte_timeout_secs = next_optional_value(
        responses_first_byte_timeout_secs,
        existing_row
            .as_ref()
            .and_then(|row| row.responses_first_byte_timeout_secs),
    );
    let next_compact_first_byte_timeout_secs = next_optional_value(
        compact_first_byte_timeout_secs,
        existing_row
            .as_ref()
            .and_then(|row| row.compact_first_byte_timeout_secs),
    );
    let next_image_first_byte_timeout_secs = next_optional_value(
        image_first_byte_timeout_secs,
        existing_row
            .as_ref()
            .and_then(|row| row.image_first_byte_timeout_secs),
    );
    let next_responses_stream_timeout_secs = next_optional_value(
        responses_stream_timeout_secs,
        existing_row
            .as_ref()
            .and_then(|row| row.responses_stream_timeout_secs),
    );
    let next_compact_stream_timeout_secs = next_optional_value(
        compact_stream_timeout_secs,
        existing_row
            .as_ref()
            .and_then(|row| row.compact_stream_timeout_secs),
    );
    let next_allow_switch_upstream = next_optional_patch_value(
        allow_switch_upstream,
        existing_row
            .as_ref()
            .and_then(|row| row.allow_switch_upstream),
    );
    let next_fast_mode_rewrite_mode = next_optional_patch_value(
        fast_mode_rewrite_mode,
        existing_row
            .as_ref()
            .and_then(|row| row.fast_mode_rewrite_mode.clone()),
    );
    let next_image_tool_rewrite_mode = next_optional_patch_value(
        image_tool_rewrite_mode,
        existing_row
            .as_ref()
            .and_then(|row| row.image_tool_rewrite_mode.clone()),
    );
    let next_codex_imagegen_rewrite_mode = next_optional_patch_value(
        codex_imagegen_rewrite_mode,
        existing_row
            .as_ref()
            .and_then(|row| row.codex_imagegen_rewrite_mode.clone()),
    );
    let next_available_models = if matches!(&available_models_mode, PatchField::Null) {
        None
    } else {
        next_optional_patch_value(
            available_models,
            existing_row
                .as_ref()
                .and_then(|row| row.available_models_json.clone()),
        )
    };
    let next_available_models_mode = next_optional_patch_value(
        available_models_mode,
        existing_row
            .as_ref()
            .and_then(|row| row.available_models_mode.clone())
            .or_else(|| {
                existing_row.as_ref().and_then(|row| {
                    row.available_models_json
                        .as_ref()
                        .map(|_| "allowlist".to_string())
                })
            }),
    );
    let next_forward_proxy_keys = next_optional_patch_value(
        forward_proxy_keys,
        existing_row
            .as_ref()
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
                forward_proxy_key,
                existing_row
                    .as_ref()
                    .and_then(|row| row.forward_proxy_key.clone()),
            )
        });
    let next_timeouts_all_clear = next_responses_first_byte_timeout_secs.is_none()
        && next_compact_first_byte_timeout_secs.is_none()
        && next_image_first_byte_timeout_secs.is_none()
        && next_responses_stream_timeout_secs.is_none()
        && next_compact_stream_timeout_secs.is_none();
    let next_policy_all_clear = next_allow_switch_upstream.is_none()
        && next_fast_mode_rewrite_mode.is_none()
        && next_image_tool_rewrite_mode.is_none()
        && next_codex_imagegen_rewrite_mode.is_none()
        && next_available_models.is_none()
        && next_available_models_mode.is_none()
        && next_forward_proxy_key.is_none()
        && next_forward_proxy_keys_json.is_none();

    match binding_kind {
        "none" => {
            if next_timeouts_all_clear && next_policy_all_clear {
                sqlx::query(
                    "DELETE FROM prompt_cache_conversation_bindings WHERE prompt_cache_key = ?1",
                )
                .bind(prompt_cache_key)
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
                        image_first_byte_timeout_secs,
                        responses_stream_timeout_secs,
                        compact_stream_timeout_secs,
                        allow_switch_upstream,
                        fast_mode_rewrite_mode,
                        image_tool_rewrite_mode,
                        codex_imagegen_rewrite_mode,
                        available_models_json,
                        available_models_mode,
                        forward_proxy_key,
                        forward_proxy_keys_json,
                        created_at,
                        updated_at
                    )
                    VALUES (?1, ?2, NULL, NULL, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, datetime('now'), datetime('now'))
                    ON CONFLICT(prompt_cache_key) DO UPDATE SET
                        binding_kind = excluded.binding_kind,
                        group_name = NULL,
                        upstream_account_id = NULL,
                        responses_first_byte_timeout_secs = excluded.responses_first_byte_timeout_secs,
                        compact_first_byte_timeout_secs = excluded.compact_first_byte_timeout_secs,
                        image_first_byte_timeout_secs = excluded.image_first_byte_timeout_secs,
                        responses_stream_timeout_secs = excluded.responses_stream_timeout_secs,
                        compact_stream_timeout_secs = excluded.compact_stream_timeout_secs,
                        allow_switch_upstream = excluded.allow_switch_upstream,
                        fast_mode_rewrite_mode = excluded.fast_mode_rewrite_mode,
                        image_tool_rewrite_mode = excluded.image_tool_rewrite_mode,
                        codex_imagegen_rewrite_mode = excluded.codex_imagegen_rewrite_mode,
                        available_models_json = excluded.available_models_json,
                        available_models_mode = excluded.available_models_mode,
                        forward_proxy_key = excluded.forward_proxy_key,
                        forward_proxy_keys_json = excluded.forward_proxy_keys_json,
                        updated_at = excluded.updated_at
                    "#,
                )
                .bind(prompt_cache_key)
                .bind(PROMPT_CACHE_BINDING_KIND_NONE)
                .bind(next_responses_first_byte_timeout_secs)
                .bind(next_compact_first_byte_timeout_secs)
                .bind(next_image_first_byte_timeout_secs)
                .bind(next_responses_stream_timeout_secs)
                .bind(next_compact_stream_timeout_secs)
                .bind(next_allow_switch_upstream)
                .bind(&next_fast_mode_rewrite_mode)
                .bind(&next_image_tool_rewrite_mode)
                .bind(&next_codex_imagegen_rewrite_mode)
                .bind(&next_available_models)
                .bind(&next_available_models_mode)
                .bind(&next_forward_proxy_key)
                .bind(&next_forward_proxy_keys_json)
                .execute(&state.pool)
                .await?;
            }
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
                    image_first_byte_timeout_secs,
                    responses_stream_timeout_secs,
                    compact_stream_timeout_secs,
                    allow_switch_upstream,
                    fast_mode_rewrite_mode,
                    image_tool_rewrite_mode,
                    codex_imagegen_rewrite_mode,
                    available_models_json,
                    available_models_mode,
                    forward_proxy_key,
                    forward_proxy_keys_json,
                    created_at,
                    updated_at
                )
                VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, datetime('now'), datetime('now'))
                ON CONFLICT(prompt_cache_key) DO UPDATE SET
                    binding_kind = excluded.binding_kind,
                    group_name = excluded.group_name,
                    upstream_account_id = NULL,
                    responses_first_byte_timeout_secs = excluded.responses_first_byte_timeout_secs,
                    compact_first_byte_timeout_secs = excluded.compact_first_byte_timeout_secs,
                    image_first_byte_timeout_secs = excluded.image_first_byte_timeout_secs,
                    responses_stream_timeout_secs = excluded.responses_stream_timeout_secs,
                    compact_stream_timeout_secs = excluded.compact_stream_timeout_secs,
                    allow_switch_upstream = excluded.allow_switch_upstream,
                    fast_mode_rewrite_mode = excluded.fast_mode_rewrite_mode,
                    image_tool_rewrite_mode = excluded.image_tool_rewrite_mode,
                    codex_imagegen_rewrite_mode = excluded.codex_imagegen_rewrite_mode,
                    available_models_json = excluded.available_models_json,
                    available_models_mode = excluded.available_models_mode,
                    forward_proxy_key = excluded.forward_proxy_key,
                    forward_proxy_keys_json = excluded.forward_proxy_keys_json,
                    updated_at = excluded.updated_at
                "#,
            )
            .bind(prompt_cache_key)
            .bind(PROMPT_CACHE_BINDING_KIND_GROUP)
            .bind(&group_name)
            .bind(next_responses_first_byte_timeout_secs)
            .bind(next_compact_first_byte_timeout_secs)
            .bind(next_image_first_byte_timeout_secs)
            .bind(next_responses_stream_timeout_secs)
            .bind(next_compact_stream_timeout_secs)
            .bind(next_allow_switch_upstream)
            .bind(&next_fast_mode_rewrite_mode)
            .bind(&next_image_tool_rewrite_mode)
            .bind(&next_codex_imagegen_rewrite_mode)
            .bind(&next_available_models)
            .bind(&next_available_models_mode)
            .bind(&next_forward_proxy_key)
            .bind(&next_forward_proxy_keys_json)
            .execute(&state.pool)
            .await?;
        }
        "upstreamAccount" => {
            let upstream_account_id = upstream_account_id.ok_or_else(|| {
                ApiError::bad_request(anyhow!(
                    "upstreamAccountId is required for upstream account binding"
                ))
            })?;
            let _ =
                ensure_upstream_account_binding_target(&state.pool, upstream_account_id).await?;
            let mut conn = state.pool.acquire().await?;
            sqlx::query("BEGIN IMMEDIATE")
                .execute(conn.as_mut())
                .await?;
            let write_result: anyhow::Result<()> = async {
                sqlx::query(
                r#"
                INSERT INTO prompt_cache_conversation_bindings (
                    prompt_cache_key,
                    binding_kind,
                    group_name,
                    upstream_account_id,
                    responses_first_byte_timeout_secs,
                    compact_first_byte_timeout_secs,
                    image_first_byte_timeout_secs,
                    responses_stream_timeout_secs,
                    compact_stream_timeout_secs,
                    allow_switch_upstream,
                    fast_mode_rewrite_mode,
                    image_tool_rewrite_mode,
                    codex_imagegen_rewrite_mode,
                    available_models_json,
                    available_models_mode,
                    forward_proxy_key,
                    forward_proxy_keys_json,
                    created_at,
                    updated_at
                )
                VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, datetime('now'), datetime('now'))
                ON CONFLICT(prompt_cache_key) DO UPDATE SET
                    binding_kind = excluded.binding_kind,
                    group_name = NULL,
                    upstream_account_id = excluded.upstream_account_id,
                    responses_first_byte_timeout_secs = excluded.responses_first_byte_timeout_secs,
                    compact_first_byte_timeout_secs = excluded.compact_first_byte_timeout_secs,
                    image_first_byte_timeout_secs = excluded.image_first_byte_timeout_secs,
                    responses_stream_timeout_secs = excluded.responses_stream_timeout_secs,
                    compact_stream_timeout_secs = excluded.compact_stream_timeout_secs,
                    allow_switch_upstream = excluded.allow_switch_upstream,
                    fast_mode_rewrite_mode = excluded.fast_mode_rewrite_mode,
                    image_tool_rewrite_mode = excluded.image_tool_rewrite_mode,
                    codex_imagegen_rewrite_mode = excluded.codex_imagegen_rewrite_mode,
                    available_models_json = excluded.available_models_json,
                    available_models_mode = excluded.available_models_mode,
                    forward_proxy_key = excluded.forward_proxy_key,
                    forward_proxy_keys_json = excluded.forward_proxy_keys_json,
                    updated_at = excluded.updated_at
                "#,
            )
            .bind(prompt_cache_key)
            .bind(PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT)
            .bind(upstream_account_id)
            .bind(next_responses_first_byte_timeout_secs)
            .bind(next_compact_first_byte_timeout_secs)
            .bind(next_image_first_byte_timeout_secs)
            .bind(next_responses_stream_timeout_secs)
            .bind(next_compact_stream_timeout_secs)
            .bind(next_allow_switch_upstream)
            .bind(&next_fast_mode_rewrite_mode)
            .bind(&next_image_tool_rewrite_mode)
            .bind(&next_codex_imagegen_rewrite_mode)
            .bind(&next_available_models)
            .bind(&next_available_models_mode)
            .bind(&next_forward_proxy_key)
            .bind(&next_forward_proxy_keys_json)
            .execute(conn.as_mut())
            .await?;
                let now_iso = format_utc_iso(Utc::now());
                overwrite_sticky_routes_for_manual_binding_executor(
                    conn.as_mut(),
                    prompt_cache_key,
                    upstream_account_id,
                    &now_iso,
                )
                .await?;
                Ok(())
            }
            .await;
            match write_result {
                Ok(()) => {
                    sqlx::query("COMMIT").execute(conn.as_mut()).await?;
                }
                Err(error) => {
                    let _ = sqlx::query("ROLLBACK").execute(conn.as_mut()).await;
                    return Err(error.into());
                }
            }
        }
        _ => {
            return Err(ApiError::bad_request(anyhow!(
                "bindingKind must be one of: none, group, upstreamAccount"
            )));
        }
    }

    let next_row =
        load_prompt_cache_conversation_binding_row(&state.pool, prompt_cache_key).await?;
    let sticky_after =
        load_prompt_cache_conversation_sticky_snapshot(&state.pool, prompt_cache_key).await?;
    let sticky_routes_after =
        load_prompt_cache_conversation_sticky_routes(&state.pool, prompt_cache_key).await?;
    let sticky_transitions =
        prompt_cache_conversation_sticky_transitions(&sticky_routes_before, &sticky_routes_after);
    let occurred_at = format_utc_iso(Utc::now());
    let binding_before =
        prompt_cache_conversation_operation_binding_snapshot_from_row(existing_row.as_ref());
    let binding_after =
        prompt_cache_conversation_operation_binding_snapshot_from_row(next_row.as_ref());
    if binding_before != binding_after {
        let action = if binding_after.binding_kind == "none" {
            "bindingCleared"
        } else {
            "manualBindingUpdated"
        };
        let input = AppendPromptCacheConversationOperationEventInput {
            prompt_cache_key: prompt_cache_key.to_string(),
            action: action.to_string(),
            origin: origin.to_string(),
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
            append_prompt_cache_conversation_operation_event(&state.pool, input).await?;
        } else {
            append_prompt_cache_conversation_operation_event_with_sticky_transitions_executor(
                &state.pool,
                input,
                &sticky_transitions,
            )
            .await?;
        }
    }

    let policy_changed_fields =
        prompt_cache_conversation_policy_changed_fields(existing_row.as_ref(), next_row.as_ref());
    if !policy_changed_fields.is_empty() {
        append_prompt_cache_conversation_operation_event(
            &state.pool,
            AppendPromptCacheConversationOperationEventInput {
                prompt_cache_key: prompt_cache_key.to_string(),
                action: "conversationPolicyUpdated".to_string(),
                origin: origin.to_string(),
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
        let (action, sticky_after_event) = match sticky_after.clone() {
            Some(snapshot) => ("stickyTargetChanged", Some(snapshot)),
            None => ("stickyTargetCleared", None),
        };
        append_prompt_cache_conversation_operation_event(
            &state.pool,
            AppendPromptCacheConversationOperationEventInput {
                prompt_cache_key: prompt_cache_key.to_string(),
                action: action.to_string(),
                origin: origin.to_string(),
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

pub(crate) async fn post_bulk_prompt_cache_conversation_bindings(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<BulkPromptCacheConversationBindingsRequest>,
) -> Result<Json<BulkPromptCacheConversationBindingsResponse>, ApiError> {
    let prompt_cache_keys = normalized_prompt_cache_conversation_keys(payload.prompt_cache_keys)?;
    match &payload.action {
        BulkPromptCacheConversationBindingsAction::Bind {
            binding_kind,
            group_name,
            upstream_account_id,
        } => {
            let trimmed_binding_kind = binding_kind.trim();
            let trimmed_group_name = group_name
                .as_ref()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty());
            if trimmed_group_name.is_some() && upstream_account_id.is_some() {
                return Err(ApiError::bad_request(anyhow!(
                    "groupName and upstreamAccountId are mutually exclusive"
                )));
            }
            match trimmed_binding_kind {
                "group" => {
                    let group_name = trimmed_group_name.ok_or_else(|| {
                        ApiError::bad_request(anyhow!("groupName is required for group binding"))
                    })?;
                    ensure_group_binding_target(&state.pool, group_name).await?;
                }
                "upstreamAccount" => {
                    let upstream_account_id = upstream_account_id.ok_or_else(|| {
                        ApiError::bad_request(anyhow!(
                            "upstreamAccountId is required for upstream account binding"
                        ))
                    })?;
                    let _ =
                        ensure_upstream_account_binding_target(&state.pool, upstream_account_id)
                            .await?;
                }
                "none" => {
                    if trimmed_group_name.is_some() || upstream_account_id.is_some() {
                        return Err(ApiError::bad_request(anyhow!(
                            "groupName and upstreamAccountId must be omitted when clearing binding"
                        )));
                    }
                }
                _ => {
                    return Err(ApiError::bad_request(anyhow!(
                        "bindingKind must be one of: none, group, upstreamAccount"
                    )));
                }
            }
        }
        BulkPromptCacheConversationBindingsAction::SetFastModeRewriteMode {
            fast_mode_rewrite_mode,
        } => {
            let normalized_mode = normalize_fast_mode_rewrite_mode(PatchField::Value(
                fast_mode_rewrite_mode.clone(),
            ))?;
            if !matches!(normalized_mode, PatchField::Value(_)) {
                return Err(ApiError::bad_request(anyhow!(
                    "fastModeRewriteMode is required"
                )));
            }
        }
        BulkPromptCacheConversationBindingsAction::ClearAndResetAffinity => {}
    }

    let mut items = Vec::with_capacity(prompt_cache_keys.len());
    let mut total_succeeded = 0usize;
    for prompt_cache_key in prompt_cache_keys {
        let result = match &payload.action {
            BulkPromptCacheConversationBindingsAction::Bind {
                binding_kind,
                group_name,
                upstream_account_id,
            } => {
                save_prompt_cache_conversation_binding_for_key(
                    state.as_ref(),
                    &prompt_cache_key,
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
                    state.as_ref(),
                    &prompt_cache_key,
                    PROMPT_CACHE_CONVERSATION_OPERATION_ORIGIN_DASHBOARD_BULK,
                )
                .await
            }
            BulkPromptCacheConversationBindingsAction::SetFastModeRewriteMode {
                fast_mode_rewrite_mode,
            } => {
                let existing_row =
                    load_prompt_cache_conversation_binding_row(&state.pool, &prompt_cache_key)
                        .await?;
                let mut request = existing_binding_request(existing_row.as_ref());
                request.fast_mode_rewrite_mode = PatchField::Value(fast_mode_rewrite_mode.clone());
                save_prompt_cache_conversation_binding_for_key(
                    state.as_ref(),
                    &prompt_cache_key,
                    request,
                    PROMPT_CACHE_CONVERSATION_OPERATION_ORIGIN_DASHBOARD_BULK,
                )
                .await
            }
        };
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
                        | ApiError::Conflict(err)
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
