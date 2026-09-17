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
const PROMPT_CACHE_CONVERSATION_OPERATION_EVENTS_DEFAULT_PAGE_SIZE: usize = 20;
const PROMPT_CACHE_CONVERSATION_OPERATION_EVENTS_MAX_PAGE_SIZE: usize = 100;

static PROMPT_CACHE_BINDING_WRITE_LOCK: once_cell::sync::Lazy<tokio::sync::Mutex<()>> =
    once_cell::sync::Lazy::new(|| tokio::sync::Mutex::new(()));

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

#[derive(Debug, Clone, FromRow)]
pub(crate) struct PromptCacheConversationOperationEventRow {
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
struct AppendPromptCacheConversationOperationEventInput {
    prompt_cache_key: String,
    action: String,
    origin: String,
    info_types: Vec<String>,
    occurred_at: String,
    headline: String,
    changed_fields: Vec<String>,
    binding_before: Option<PromptCacheConversationOperationBindingSnapshot>,
    binding_after: Option<PromptCacheConversationOperationBindingSnapshot>,
    sticky_before: Option<PromptCacheConversationOperationStickySnapshot>,
    sticky_after: Option<PromptCacheConversationOperationStickySnapshot>,
    invoke_id: Option<String>,
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
