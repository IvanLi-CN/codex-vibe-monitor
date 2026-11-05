use super::*;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountListMetrics {
    pub(crate) total: usize,
    pub(crate) oauth: usize,
    pub(crate) api_key: usize,
    pub(crate) attention: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountListResponse {
    pub(crate) writes_enabled: bool,
    pub(crate) items: Vec<UpstreamAccountSummary>,
    pub(crate) total: usize,
    pub(crate) page: usize,
    pub(crate) page_size: usize,
    pub(crate) metrics: UpstreamAccountListMetrics,
    pub(crate) groups: Vec<UpstreamAccountGroupSummary>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) forward_proxy_nodes: Vec<ForwardProxyBindingNodeResponse>,
    pub(crate) has_ungrouped_accounts: bool,
    pub(crate) routing: PoolRoutingSettingsResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountActionEventListResponse {
    pub(crate) items: Vec<UpstreamAccountActionEvent>,
    pub(crate) total: usize,
    pub(crate) page: usize,
    pub(crate) page_size: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelRoutingLiveResponse {
    pub(crate) generated_at: String,
    pub(crate) groups: Vec<ModelRoutingLiveModelGroup>,
    pub(crate) records: Vec<ModelRoutingTimelineRecord>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelRoutingLiveModelGroup {
    pub(crate) model: String,
    pub(crate) accounts: Vec<ModelRoutingLiveAccount>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelRoutingLiveAccount {
    pub(crate) account_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) account_display_name: Option<String>,
    #[serde(flatten)]
    pub(crate) route: ModelRoutingState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelRoutingTimelineRecord {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) occurred_at: String,
    pub(crate) account_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) account_display_name: Option<String>,
    pub(crate) model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) attempt_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) invoke_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) attempt_index: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) same_account_retry_index: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) routing_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) routing_selection_audit: Option<PoolRoutingSelectionAudit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) http_status: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) failure_kind: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_opt_finite_positive_timing"
    )]
    pub(crate) total_latency_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reason_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) model_route_state_before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) model_route_state_after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) model_route_priority_before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) model_route_priority_after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) model_route_failure_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) model_route_cooldown_until: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelRoutingHistoryResponse {
    pub(crate) items: Vec<ModelRoutingTimelineRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_cursor: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelRoutingLiveQuery {
    pub(crate) window: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) state: Option<String>,
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelRoutingHistoryQuery {
    pub(crate) model: String,
    pub(crate) cursor: Option<String>,
    pub(crate) page_size: Option<usize>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountAttemptStickyKeyOption {
    pub(crate) value: String,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) latest_created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountAttemptListResponse {
    pub(crate) items: Vec<ApiPoolUpstreamRequestAttempt>,
    pub(crate) sticky_key_options: Vec<UpstreamAccountAttemptStickyKeyOption>,
    pub(crate) total: usize,
    pub(crate) page: usize,
    pub(crate) page_size: usize,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocateUpstreamAccountAttemptQuery {
    pub(crate) attempt_id: String,
    pub(crate) page_size: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListUpstreamAccountAttemptsQuery {
    #[serde(rename = "type")]
    pub(crate) attempt_type: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) sticky_key: Option<String>,
    pub(crate) page: Option<usize>,
    pub(crate) page_size: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountWindowUsageRequest {
    #[serde(default)]
    pub(crate) account_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountWindowUsageItem {
    pub(crate) account_id: i64,
    pub(crate) primary_actual_usage: Option<RateWindowActualUsage>,
    pub(crate) secondary_actual_usage: Option<RateWindowActualUsage>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountWindowUsageResponse {
    pub(crate) items: Vec<UpstreamAccountWindowUsageItem>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListForwardProxyBindingNodesQuery {
    #[serde(default)]
    pub(crate) key: Vec<String>,
    #[serde(default)]
    pub(crate) include_current: bool,
    pub(crate) group_name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListUpstreamAccountActionEventsQuery {
    pub(crate) kind: Option<String>,
    pub(crate) account: Option<String>,
    pub(crate) group: Option<String>,
    pub(crate) proxy_key: Option<String>,
    pub(crate) action: Option<String>,
    pub(crate) result: Option<String>,
    pub(crate) page: Option<usize>,
    pub(crate) page_size: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListUpstreamAccountsQuery {
    pub(crate) kind: Option<String>,
    #[serde(default)]
    pub(crate) group_exact: Vec<String>,
    pub(crate) group_search: Option<String>,
    pub(crate) group_ungrouped: Option<bool>,
    pub(crate) status: Option<String>,
    #[serde(default)]
    pub(crate) work_status: Vec<String>,
    #[serde(default)]
    pub(crate) enable_status: Vec<String>,
    #[serde(default)]
    pub(crate) health_status: Vec<String>,
    pub(crate) page: Option<usize>,
    pub(crate) page_size: Option<usize>,
    pub(crate) include_all: Option<bool>,
    #[serde(default)]
    pub(crate) tag_ids: Vec<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListUpstreamAccountsBaseQuery {
    pub(crate) group_search: Option<String>,
    pub(crate) group_ungrouped: Option<bool>,
    pub(crate) status: Option<String>,
    pub(crate) page: Option<usize>,
    pub(crate) page_size: Option<usize>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DuplicateReason {
    SharedChatgptAccountId,
    SharedChatgptUserId,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub(crate) enum TagPriorityTier {
    NoNew,
    Fallback,
    #[default]
    Normal,
    Primary,
}

impl TagPriorityTier {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NoNew => "no_new",
            Self::Fallback => "fallback",
            Self::Normal => "normal",
            Self::Primary => "primary",
        }
    }

    pub(crate) fn routing_rank(self) -> u8 {
        match self {
            Self::Primary => 0,
            Self::Normal => 1,
            Self::Fallback => 2,
            Self::NoNew => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TagFastModeRewriteMode {
    ForceRemove,
    #[default]
    KeepOriginal,
    FillMissing,
    ForceAdd,
}

impl TagFastModeRewriteMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ForceRemove => "force_remove",
            Self::KeepOriginal => "keep_original",
            Self::FillMissing => "fill_missing",
            Self::ForceAdd => "force_add",
        }
    }

    pub(crate) fn merge_rank(self) -> u8 {
        match self {
            Self::ForceRemove => 0,
            Self::ForceAdd => 1,
            Self::FillMissing => 2,
            Self::KeepOriginal => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ImageToolRewriteMode {
    ForceRemove,
    #[default]
    KeepOriginal,
    FillMissing,
    ForceAdd,
}

impl ImageToolRewriteMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ForceRemove => "force_remove",
            Self::KeepOriginal => "keep_original",
            Self::FillMissing => "fill_missing",
            Self::ForceAdd => "force_add",
        }
    }

    pub(crate) fn from_str(value: &str) -> Self {
        match value.trim() {
            "force_remove" => Self::ForceRemove,
            "fill_missing" => Self::FillMissing,
            "force_add" => Self::ForceAdd,
            _ => Self::KeepOriginal,
        }
    }
}

/// Controls the client-executed `image_gen.imagegen` tool advertised to Codex.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CodexImagegenRewriteMode {
    ForceRemove,
    #[default]
    KeepOriginal,
    FillMissing,
    ForceAdd,
}

impl CodexImagegenRewriteMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ForceRemove => "force_remove",
            Self::KeepOriginal => "keep_original",
            Self::FillMissing => "fill_missing",
            Self::ForceAdd => "force_add",
        }
    }

    pub(crate) fn from_str(value: &str) -> Self {
        match value.trim() {
            "force_remove" => Self::ForceRemove,
            "fill_missing" => Self::FillMissing,
            "force_add" => Self::ForceAdd,
            _ => Self::KeepOriginal,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RequestCompressionAlgorithm {
    Follow,
    #[default]
    Identity,
    Gzip,
    Deflate,
    Zstd,
}

impl RequestCompressionAlgorithm {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Follow => "follow",
            Self::Identity => "identity",
            Self::Gzip => "gzip",
            Self::Deflate => "deflate",
            Self::Zstd => "zstd",
        }
    }

    pub(crate) fn from_str(value: &str) -> Self {
        match value.trim() {
            "follow" => Self::Follow,
            "gzip" => Self::Gzip,
            "deflate" => Self::Deflate,
            "zstd" => Self::Zstd,
            _ => Self::Identity,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RequestCompressionLevelPreset {
    Fast,
    #[default]
    Balanced,
    Best,
}

impl RequestCompressionLevelPreset {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Balanced => "balanced",
            Self::Best => "best",
        }
    }

    pub(crate) fn from_str(value: &str) -> Self {
        match value.trim() {
            "fast" => Self::Fast,
            "best" => Self::Best,
            _ => Self::Balanced,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CapabilitySupport {
    Supported,
    Unsupported,
    #[default]
    Unknown,
}

impl CapabilitySupport {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Unsupported => "unsupported",
            Self::Unknown => "unknown",
        }
    }

    pub(crate) fn from_str(value: &str) -> Self {
        match value.trim() {
            "supported" => Self::Supported,
            "unsupported" => Self::Unsupported,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamCapabilityState {
    pub(crate) observed: CapabilitySupport,
    #[serde(rename = "override")]
    pub(crate) override_value: Option<CapabilitySupport>,
    pub(crate) effective: CapabilitySupport,
    pub(crate) observed_at: Option<String>,
    pub(crate) reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ImageIntent {
    Yes,
    DirectImage,
    No,
    #[default]
    Unknown,
}

impl ImageIntent {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::DirectImage => "direct_image",
            Self::No => "no",
            Self::Unknown => "unknown",
        }
    }

    pub(crate) fn from_str(value: &str) -> Self {
        match value.trim() {
            "yes" => Self::Yes,
            "direct_image" => Self::DirectImage,
            "no" => Self::No,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RequestCapabilityRequirements {
    pub(crate) response_endpoint: bool,
    pub(crate) chat_completions_endpoint: bool,
    pub(crate) image_endpoint: bool,
    pub(crate) response_image_tool: bool,
    pub(crate) codex_imagegen: bool,
    pub(crate) standalone_search: bool,
}

impl RequestCapabilityRequirements {
    pub(crate) fn from_endpoint_and_image_intent(
        endpoint: &str,
        image_intent: ImageIntent,
    ) -> Self {
        match endpoint {
            "/v1/responses" | "/v1/responses/compact" => Self::response_family(image_intent),
            "/v1/chat/completions" => Self::chat_completions(),
            "/v1/images/generations" | "/v1/images/edits" => Self::direct_image_endpoint(),
            "/v1/alpha/search" => Self::standalone_search(),
            _ => Self::default(),
        }
    }

    pub(crate) fn from_endpoint_and_image_intent_for_method(
        endpoint: &str,
        image_intent: ImageIntent,
        is_post: bool,
    ) -> Self {
        let mut requirements = Self::from_endpoint_and_image_intent(endpoint, image_intent);
        if !is_post {
            requirements.standalone_search = false;
        }
        requirements
    }

    pub(crate) fn direct_image_endpoint() -> Self {
        Self {
            response_endpoint: false,
            chat_completions_endpoint: false,
            image_endpoint: true,
            response_image_tool: false,
            codex_imagegen: false,
            standalone_search: false,
        }
    }

    pub(crate) fn chat_completions() -> Self {
        Self {
            response_endpoint: false,
            chat_completions_endpoint: true,
            image_endpoint: false,
            response_image_tool: false,
            codex_imagegen: false,
            standalone_search: false,
        }
    }

    pub(crate) fn response_family(image_intent: ImageIntent) -> Self {
        Self {
            response_endpoint: true,
            chat_completions_endpoint: false,
            image_endpoint: false,
            response_image_tool: matches!(image_intent, ImageIntent::Yes),
            codex_imagegen: false,
            standalone_search: false,
        }
    }

    pub(crate) fn standalone_search() -> Self {
        Self {
            standalone_search: true,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DuplicateInfo {
    pub(crate) peer_account_ids: Vec<i64>,
    pub(crate) reasons: Vec<DuplicateReason>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountTagSummary {
    pub(crate) id: i64,
    pub(crate) name: String,
    pub(crate) routing_rule: TagRoutingRule,
    pub(crate) available_models_invalid: bool,
    pub(crate) system_key: Option<String>,
    pub(crate) protected: bool,
}
