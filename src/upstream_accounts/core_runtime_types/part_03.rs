#[derive(Debug, Clone, Default)]
pub(crate) struct ConversationRoutingOverride {
    pub(crate) allow_switch_upstream: Option<bool>,
    pub(crate) fast_mode_rewrite_mode: Option<TagFastModeRewriteMode>,
    pub(crate) image_tool_rewrite_mode: Option<ImageToolRewriteMode>,
    pub(crate) codex_imagegen_rewrite_mode: Option<CodexImagegenRewriteMode>,
    pub(crate) available_models: Option<Vec<String>>,
    pub(crate) available_models_mode: Option<AvailableModelsMode>,
    pub(crate) available_models_invalid: bool,
    pub(crate) forward_proxy_key: Option<String>,
    pub(crate) forward_proxy_keys: Vec<String>,
    pub(crate) forward_proxy_scope_key: String,
}

impl ConversationRoutingOverride {
    pub(crate) fn has_policy_override(&self) -> bool {
        self.allow_switch_upstream.is_some()
            || self.fast_mode_rewrite_mode.is_some()
            || self.image_tool_rewrite_mode.is_some()
            || self.codex_imagegen_rewrite_mode.is_some()
            || self.available_models.is_some()
            || self.available_models_mode.is_some()
            || self.available_models_invalid
            || self.forward_proxy_key.is_some()
            || !self.forward_proxy_keys.is_empty()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TagRoutingRule {
    pub(crate) allow_cut_out: bool,
    pub(crate) allow_cut_in: bool,
    pub(crate) priority_tier: TagPriorityTier,
    pub(crate) fast_mode_rewrite_mode: TagFastModeRewriteMode,
    pub(crate) concurrency_limit: i64,
    pub(crate) upstream_429_retry_enabled: bool,
    pub(crate) upstream_429_max_retries: u8,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) available_models: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GroupAccountRoutingRule {
    pub(crate) allow_cut_out: bool,
    pub(crate) allow_cut_in: bool,
    pub(crate) priority_tier: TagPriorityTier,
    pub(crate) fast_mode_rewrite_mode: TagFastModeRewriteMode,
    pub(crate) image_tool_rewrite_mode: ImageToolRewriteMode,
    pub(crate) codex_imagegen_rewrite_mode: Option<CodexImagegenRewriteMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) request_compression_algorithm: Option<RequestCompressionAlgorithm>,
    pub(crate) concurrency_limit: i64,
    pub(crate) upstream_429_retry_enabled: bool,
    pub(crate) upstream_429_max_retries: u8,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) available_models: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) available_models_mode: Option<AvailableModelsMode>,
    pub(crate) available_models_defined: bool,
    pub(crate) status_change_reasons: StatusChangeReasonSettings,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) timeouts: Option<RoutingTimeoutSettings>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TagSummary {
    pub(crate) id: i64,
    pub(crate) name: String,
    pub(crate) routing_rule: TagRoutingRule,
    pub(crate) account_count: i64,
    pub(crate) group_count: i64,
    pub(crate) updated_at: String,
    pub(crate) system_key: Option<String>,
    pub(crate) protected: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TagDetail {
    #[serde(flatten)]
    pub(crate) summary: TagSummary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TagListResponse {
    pub(crate) writes_enabled: bool,
    pub(crate) items: Vec<TagSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountGroupSummary {
    pub(crate) group_name: String,
    pub(crate) account_count: i64,
    pub(crate) note: Option<String>,
    pub(crate) bound_proxy_keys: Vec<String>,
    pub(crate) node_shunt_enabled: bool,
    pub(crate) single_account_rotation_enabled: bool,
    pub(crate) upstream_429_retry_enabled: bool,
    pub(crate) upstream_429_max_retries: u8,
    pub(crate) concurrency_limit: i64,
    pub(crate) routing_rule: GroupAccountRoutingRule,
    pub(crate) effective_timeouts: RoutingTimeoutSettings,
    pub(crate) timeout_field_sources: RoutingTimeoutFieldSources,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct UpstreamAccountGroupMetadata {
    pub(crate) note: Option<String>,
    pub(crate) bound_proxy_keys: Vec<String>,
    pub(crate) node_shunt_enabled: bool,
    pub(crate) single_account_rotation_enabled: bool,
    pub(crate) upstream_429_retry_enabled: bool,
    pub(crate) upstream_429_max_retries: u8,
    pub(crate) concurrency_limit: i64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RequestedGroupMetadataChanges {
    pub(crate) note: Option<String>,
    pub(crate) note_was_requested: bool,
    pub(crate) bound_proxy_keys: Vec<String>,
    pub(crate) bound_proxy_keys_was_requested: bool,
    pub(crate) concurrency_limit: i64,
    pub(crate) concurrency_limit_was_requested: bool,
    pub(crate) node_shunt_enabled: bool,
    pub(crate) node_shunt_enabled_was_requested: bool,
    pub(crate) single_account_rotation_enabled: bool,
    pub(crate) single_account_rotation_enabled_was_requested: bool,
}

impl RequestedGroupMetadataChanges {
    pub(crate) fn was_requested(&self) -> bool {
        self.note_was_requested
            || self.bound_proxy_keys_was_requested
            || self.concurrency_limit_was_requested
            || self.node_shunt_enabled_was_requested
            || self.single_account_rotation_enabled_was_requested
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedRequiredGroupProxyBinding {
    pub(crate) group_name: String,
    pub(crate) bound_proxy_keys: Vec<String>,
    pub(crate) node_shunt_enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountSummary {
    pub(crate) id: i64,
    pub(crate) kind: String,
    pub(crate) provider: String,
    pub(crate) display_name: String,
    pub(crate) group_name: Option<String>,
    pub(crate) is_mother: bool,
    pub(crate) status: String,
    pub(crate) display_status: String,
    pub(crate) enabled: bool,
    pub(crate) work_status: String,
    pub(crate) enable_status: String,
    pub(crate) health_status: String,
    pub(crate) sync_state: String,
    pub(crate) email: Option<String>,
    pub(crate) chatgpt_account_id: Option<String>,
    pub(crate) plan_type: Option<String>,
    pub(crate) masked_api_key: Option<String>,
    pub(crate) has_refresh_token: bool,
    pub(crate) last_synced_at: Option<String>,
    pub(crate) last_successful_sync_at: Option<String>,
    pub(crate) last_activity_at: Option<String>,
    pub(crate) active_conversation_count: i64,
    pub(crate) last_error: Option<String>,
    pub(crate) last_error_at: Option<String>,
    pub(crate) last_action: Option<String>,
    pub(crate) last_action_source: Option<String>,
    pub(crate) last_action_reason_code: Option<String>,
    pub(crate) last_action_reason_message: Option<String>,
    pub(crate) last_action_http_status: Option<u16>,
    pub(crate) last_action_invoke_id: Option<String>,
    pub(crate) last_action_at: Option<String>,
    pub(crate) cooldown_until: Option<String>,
    pub(crate) bound_proxy_keys: Vec<String>,
    pub(crate) current_forward_proxy_key: Option<String>,
    pub(crate) current_forward_proxy_display_name: Option<String>,
    pub(crate) current_forward_proxy_state: String,
    pub(crate) routing_block_reason_code: Option<String>,
    pub(crate) routing_block_reason_message: Option<String>,
    pub(crate) routing_block_until: Option<String>,
    pub(crate) token_expires_at: Option<String>,
    pub(crate) primary_window: Option<RateWindowSnapshot>,
    pub(crate) secondary_window: Option<RateWindowSnapshot>,
    pub(crate) credits: Option<CreditsSnapshot>,
    pub(crate) local_limits: Option<LocalLimitSnapshot>,
    pub(crate) compact_support: CompactSupportState,
    pub(crate) duplicate_info: Option<DuplicateInfo>,
    pub(crate) tags: Vec<AccountTagSummary>,
    pub(crate) effective_routing_rule: EffectiveRoutingRule,
    pub(crate) response_endpoint_capability: UpstreamCapabilityState,
    pub(crate) chat_completions_capability: UpstreamCapabilityState,
    pub(crate) image_endpoint_capability: UpstreamCapabilityState,
    pub(crate) response_image_tool_capability: UpstreamCapabilityState,
    pub(crate) codex_imagegen_capability: UpstreamCapabilityState,
    pub(crate) standalone_search_capability: UpstreamCapabilityState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountDetail {
    #[serde(flatten)]
    pub(crate) summary: UpstreamAccountSummary,
    pub(crate) note: Option<String>,
    pub(crate) upstream_base_url: Option<String>,
    pub(crate) chatgpt_user_id: Option<String>,
    pub(crate) verified_email: Option<String>,
    pub(crate) last_refreshed_at: Option<String>,
    pub(crate) history: Vec<UpstreamAccountHistoryPoint>,
    pub(crate) recent_actions: Vec<UpstreamAccountActionEvent>,
    pub(crate) model_mappings: Vec<ModelMapping>,
    pub(crate) model_catalog: UpstreamAccountModelCatalog,
    pub(crate) model_routing_states: Vec<ModelRoutingState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) routing_state_version: Option<RoutingStateVersion>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountModelCatalog {
    pub(crate) models: Vec<String>,
    pub(crate) status: String,
    pub(crate) last_attempted_at: Option<String>,
    pub(crate) last_successful_at: Option<String>,
    pub(crate) error: Option<UpstreamAccountModelCatalogError>,
    pub(crate) stale: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountModelCatalogError {
    pub(crate) code: String,
    pub(crate) message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelRoutingState {
    pub(crate) model: String,
    pub(crate) state: String,
    pub(crate) priority: String,
    pub(crate) failure_count: i64,
    pub(crate) changed_at: Option<String>,
    pub(crate) last_seen_at: String,
    pub(crate) last_failure_at: Option<String>,
    pub(crate) last_failure_kind: Option<String>,
    #[serde(skip_serializing)]
    pub(crate) last_failure_message: Option<String>,
    pub(crate) cooldown_until: Option<String>,
    pub(crate) cache_concurrency_limit: Option<i64>,
    pub(crate) cache_recovery_limit: Option<i64>,
    pub(crate) cache_low_hit_streak: i64,
    pub(crate) cache_cooldown_level: i64,
    pub(crate) cache_last_hit_rate_percent: Option<i64>,
    pub(crate) cache_usage_missing_since: Option<String>,
    pub(crate) cache_usage_missing_reason: Option<String>,
    pub(crate) probe_required: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResetModelRoutingRequest {
    pub(crate) model: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateModelMappingsRequest {
    #[serde(default)]
    pub(crate) model_mappings: Vec<ModelMapping>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountActionEvent {
    pub(crate) id: i64,
    pub(crate) occurred_at: String,
    pub(crate) action: String,
    pub(crate) source: String,
    pub(crate) account_display_name: Option<String>,
    pub(crate) account_group_name: Option<String>,
    pub(crate) forward_proxy_key: Option<String>,
    pub(crate) forward_proxy_display_name: Option<String>,
    pub(crate) forward_proxy_egress_ip: Option<String>,
    pub(crate) result: Option<String>,
    pub(crate) result_description: Option<String>,
    pub(crate) reason_code: Option<String>,
    pub(crate) reason_message: Option<String>,
    pub(crate) http_status: Option<u16>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) invoke_id: Option<String>,
    pub(crate) attempt_id: Option<String>,
    pub(crate) sticky_key: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) model_route_state_before: Option<String>,
    pub(crate) model_route_state_after: Option<String>,
    pub(crate) model_route_priority_before: Option<String>,
    pub(crate) model_route_priority_after: Option<String>,
    pub(crate) model_route_failure_count: Option<i64>,
    pub(crate) model_route_cooldown_until: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) blocked_binding: Option<BlockedBindingDiagnostic>,
    pub(crate) created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompactSupportState {
    pub(crate) status: String,
    pub(crate) observed_at: Option<String>,
    pub(crate) reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PoolRoutingTimeoutSettingsResponse {
    pub(crate) responses_first_byte_timeout_secs: u64,
    pub(crate) compact_first_byte_timeout_secs: u64,
    pub(crate) image_first_byte_timeout_secs: u64,
    pub(crate) responses_stream_timeout_secs: u64,
    pub(crate) compact_stream_timeout_secs: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RoutingTimeoutOverridesResolved {
    pub(crate) responses_first_byte_timeout: Option<Duration>,
    pub(crate) compact_first_byte_timeout: Option<Duration>,
    pub(crate) image_first_byte_timeout: Option<Duration>,
    pub(crate) responses_stream_timeout: Option<Duration>,
    pub(crate) compact_stream_timeout: Option<Duration>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PoolRoutingTimeoutSettingsResolved {
    pub(crate) default_first_byte_timeout: Duration,
    pub(crate) default_send_timeout: Duration,
    pub(crate) request_read_timeout: Duration,
    pub(crate) responses_first_byte_timeout: Duration,
    pub(crate) compact_first_byte_timeout: Duration,
    pub(crate) image_first_byte_timeout: Duration,
    pub(crate) responses_stream_timeout: Duration,
    pub(crate) compact_stream_timeout: Duration,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PoolRoutingRequestCompressionSettingsResolved {
    pub(crate) algorithm: RequestCompressionAlgorithm,
    pub(crate) level_preset: RequestCompressionLevelPreset,
}

impl PoolRoutingTimeoutSettingsResolved {
    pub(crate) fn with_overrides(
        self,
        overrides: RoutingTimeoutOverridesResolved,
    ) -> PoolRoutingTimeoutSettingsResolved {
        PoolRoutingTimeoutSettingsResolved {
            default_first_byte_timeout: self.default_first_byte_timeout,
            default_send_timeout: self.default_send_timeout,
            request_read_timeout: self.request_read_timeout,
            responses_first_byte_timeout: overrides
                .responses_first_byte_timeout
                .unwrap_or(self.responses_first_byte_timeout),
            compact_first_byte_timeout: overrides
                .compact_first_byte_timeout
                .unwrap_or(self.compact_first_byte_timeout),
            image_first_byte_timeout: overrides
                .image_first_byte_timeout
                .unwrap_or(self.image_first_byte_timeout),
            responses_stream_timeout: overrides
                .responses_stream_timeout
                .unwrap_or(self.responses_stream_timeout),
            compact_stream_timeout: overrides
                .compact_stream_timeout
                .unwrap_or(self.compact_stream_timeout),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PoolRoutingSettingsResponse {
    pub(crate) writes_enabled: bool,
    pub(crate) api_key_configured: bool,
    pub(crate) masked_api_key: Option<String>,
    pub(crate) maintenance: PoolRoutingMaintenanceSettingsResponse,
    pub(crate) request_compression_algorithm: RequestCompressionAlgorithm,
    pub(crate) request_compression_level_preset: RequestCompressionLevelPreset,
    pub(crate) codex_imagegen_rewrite_mode: CodexImagegenRewriteMode,
    pub(crate) available_models: Vec<String>,
    pub(crate) available_models_mode: AvailableModelsMode,
    pub(crate) timeouts: PoolRoutingTimeoutSettingsResponse,
    pub(crate) cache_hit_protection: CacheHitProtectionSettingsResponse,
    pub(crate) priority_handoff_admission_enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CacheHitProtectionSettingsResponse {
    pub(crate) enabled: bool,
    pub(crate) low_hit_rate_threshold_percent: u8,
    pub(crate) overflow_mode: String,
    pub(crate) minimum_input_tokens: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PoolRoutingMaintenanceSettingsResponse {
    pub(crate) primary_sync_interval_secs: u64,
    pub(crate) secondary_sync_interval_secs: u64,
    pub(crate) priority_available_account_cap: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdatePoolRoutingSettingsRequest {
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) maintenance: Option<UpdatePoolRoutingMaintenanceSettingsRequest>,
    #[serde(default)]
    pub(crate) request_compression_algorithm: Option<String>,
    #[serde(default)]
    pub(crate) request_compression_level_preset: Option<String>,
    #[serde(default)]
    pub(crate) codex_imagegen_rewrite_mode: Option<String>,
    #[serde(default)]
    pub(crate) available_models: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) available_models_mode: Option<String>,
    #[serde(default)]
    pub(crate) timeouts: Option<UpdatePoolRoutingTimeoutSettingsRequest>,
    #[serde(default)]
    pub(crate) cache_hit_protection: Option<UpdateCacheHitProtectionSettingsRequest>,
    #[serde(default)]
    pub(crate) priority_handoff_admission_enabled: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateCacheHitProtectionSettingsRequest {
    #[serde(default)]
    pub(crate) enabled: Option<bool>,
    #[serde(default)]
    pub(crate) low_hit_rate_threshold_percent: Option<u8>,
    #[serde(default)]
    pub(crate) overflow_mode: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateRoutingTimeoutSettingsRequest {
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) responses_first_byte_timeout_secs: OptionalField<u64>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) compact_first_byte_timeout_secs: OptionalField<u64>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) image_first_byte_timeout_secs: OptionalField<u64>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) responses_stream_timeout_secs: OptionalField<u64>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) compact_stream_timeout_secs: OptionalField<u64>,
}

impl UpdateRoutingTimeoutSettingsRequest {
    pub(crate) fn is_empty(&self) -> bool {
        matches!(
            self.responses_first_byte_timeout_secs,
            OptionalField::Missing
        ) && matches!(self.compact_first_byte_timeout_secs, OptionalField::Missing)
            && matches!(self.image_first_byte_timeout_secs, OptionalField::Missing)
            && matches!(self.responses_stream_timeout_secs, OptionalField::Missing)
            && matches!(self.compact_stream_timeout_secs, OptionalField::Missing)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdatePoolRoutingMaintenanceSettingsRequest {
    #[serde(default)]
    pub(crate) primary_sync_interval_secs: Option<u64>,
    #[serde(default)]
    pub(crate) secondary_sync_interval_secs: Option<u64>,
    #[serde(default)]
    pub(crate) priority_available_account_cap: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdatePoolRoutingTimeoutSettingsRequest {
    #[serde(default)]
    pub(crate) responses_first_byte_timeout_secs: Option<u64>,
    #[serde(default)]
    #[serde(alias = "compactUpstreamHandshakeTimeoutSecs")]
    pub(crate) compact_first_byte_timeout_secs: Option<u64>,
    #[serde(default)]
    pub(crate) image_first_byte_timeout_secs: Option<u64>,
    #[serde(default)]
    pub(crate) responses_stream_timeout_secs: Option<u64>,
    #[serde(default)]
    pub(crate) compact_stream_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountStickyKeysResponse {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) selection_mode: AccountStickyKeySelectionMode,
    pub(crate) selected_limit: Option<i64>,
    pub(crate) selected_activity_hours: Option<i64>,
    pub(crate) implicit_filter: AccountStickyKeyImplicitFilter,
    pub(crate) conversations: Vec<AccountStickyKeyConversation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AccountStickyKeySelectionMode {
    Count,
    ActivityWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccountStickyKeySelection {
    Count(i64),
    ActivityWindow(i64),
}

impl AccountStickyKeySelection {
    pub(crate) fn selection_mode(self) -> AccountStickyKeySelectionMode {
        match self {
            Self::Count(_) => AccountStickyKeySelectionMode::Count,
            Self::ActivityWindow(_) => AccountStickyKeySelectionMode::ActivityWindow,
        }
    }

    pub(crate) fn selected_limit(self) -> Option<i64> {
        match self {
            Self::Count(limit) => Some(limit),
            Self::ActivityWindow(_) => None,
        }
    }

    pub(crate) fn selected_activity_hours(self) -> Option<i64> {
        match self {
            Self::Count(_) => None,
            Self::ActivityWindow(hours) => Some(hours),
        }
    }

    pub(crate) fn activity_window_hours(self) -> i64 {
        match self {
            Self::Count(_) => 24,
            Self::ActivityWindow(hours) => hours,
        }
    }

    pub(crate) fn display_limit(self) -> i64 {
        match self {
            Self::Count(limit) => limit,
            Self::ActivityWindow(_) => STICKY_KEY_ACTIVITY_MODE_LIMIT,
        }
    }

    pub(crate) fn implicit_filter(
        self,
        filtered_counts: AccountStickyKeyFilteredCounts,
    ) -> AccountStickyKeyImplicitFilter {
        let (kind, filtered_count) = match self {
            Self::Count(_) => (None, 0),
            Self::ActivityWindow(_) if filtered_counts.capped_count > 0 => (
                Some(AccountStickyKeyImplicitFilterKind::CappedTo50),
                filtered_counts.capped_count,
            ),
            Self::ActivityWindow(_) if filtered_counts.inactive_count > 0 => (
                Some(AccountStickyKeyImplicitFilterKind::InactiveOutside24h),
                filtered_counts.inactive_count,
            ),
            Self::ActivityWindow(_) => (None, 0),
        };
        AccountStickyKeyImplicitFilter {
            kind,
            filtered_count: filtered_count.max(0),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AccountStickyKeyFilteredCounts {
    pub(crate) inactive_count: i64,
    pub(crate) capped_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountStickyKeyImplicitFilter {
    pub(crate) kind: Option<AccountStickyKeyImplicitFilterKind>,
    pub(crate) filtered_count: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AccountStickyKeyImplicitFilterKind {
    InactiveOutside24h,
    CappedTo50,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountStickyKeyConversation {
    pub(crate) sticky_key: String,
    pub(crate) request_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) created_at: String,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) last_activity_at: String,
    pub(crate) recent_invocations:
        Vec<crate::api::PromptCacheConversationInvocationPreviewResponse>,
    pub(crate) last24h_requests: Vec<AccountStickyKeyRequestPoint>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountStickyKeyRequestPoint {
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) occurred_at: String,
    pub(crate) status: String,
    pub(crate) is_success: bool,
    pub(crate) request_tokens: i64,
    pub(crate) cumulative_tokens: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountHistoryPoint {
    pub(crate) captured_at: String,
    pub(crate) primary_used_percent: Option<f64>,
    pub(crate) secondary_used_percent: Option<f64>,
    pub(crate) credits_balance: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RateWindowSnapshot {
    pub(crate) used_percent: f64,
    pub(crate) used_text: String,
    pub(crate) limit_text: String,
    pub(crate) resets_at: Option<String>,
    pub(crate) window_duration_mins: i64,
    pub(crate) actual_usage: Option<RateWindowActualUsage>,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RateWindowActualUsage {
    pub(crate) request_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) cache_input_tokens: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreditsSnapshot {
    pub(crate) has_credits: bool,
    pub(crate) unlimited: bool,
    pub(crate) balance: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalLimitSnapshot {
    pub(crate) primary_limit: Option<f64>,
    pub(crate) secondary_limit: Option<f64>,
    pub(crate) limit_unit: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoginSessionStatusResponse {
    pub(crate) login_id: String,
    pub(crate) status: String,
    pub(crate) auth_url: Option<String>,
    pub(crate) redirect_uri: Option<String>,
    pub(crate) expires_at: String,
    pub(crate) updated_at: String,
    pub(crate) account_id: Option<i64>,
    pub(crate) email: Option<String>,
    pub(crate) error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sync_applied: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) identity_confirmation: Option<OauthIdentityConfirmationResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthIdentityConfirmationResponse {
    pub(crate) current: OauthIdentitySummaryResponse,
    pub(crate) incoming: OauthIdentitySummaryResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthIdentitySummaryResponse {
    pub(crate) account_id: Option<i64>,
    pub(crate) display_name: Option<String>,
    pub(crate) email: Option<String>,
    pub(crate) verified_email: Option<String>,
    pub(crate) chatgpt_account_id: Option<String>,
    pub(crate) chatgpt_user_id: Option<String>,
    pub(crate) plan_type: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthMailboxSessionResponse {
    pub(crate) email_address: String,
    pub(crate) supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthMailboxCodeSummary {
    pub(crate) value: String,
    pub(crate) source: String,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthInviteSummary {
    pub(crate) subject: String,
    pub(crate) copy_value: String,
    pub(crate) copy_label: String,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthMailboxStatus {
    pub(crate) session_id: String,
    pub(crate) email_address: String,
    pub(crate) expires_at: String,
    pub(crate) latest_code: Option<OauthMailboxCodeSummary>,
    pub(crate) invite: Option<OauthInviteSummary>,
    pub(crate) invited: bool,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthMailboxStatusBatchResponse {
    pub(crate) items: Vec<OauthMailboxStatus>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateOauthLoginSessionRequest {
    pub(crate) display_name: Option<String>,
    pub(crate) email: Option<String>,
    pub(crate) group_name: Option<String>,
    #[serde(default)]
    pub(crate) group_bound_proxy_keys: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) group_node_shunt_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) group_single_account_rotation_enabled: Option<bool>,
    pub(crate) note: Option<String>,
    pub(crate) group_note: Option<String>,
    pub(crate) concurrency_limit: Option<i64>,
    pub(crate) account_id: Option<i64>,
    #[serde(default)]
    pub(crate) tag_ids: Vec<i64>,
    pub(crate) is_mother: Option<bool>,
    pub(crate) mailbox_session_id: Option<String>,
    #[serde(alias = "generatedMailboxAddress")]
    pub(crate) mailbox_address: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompleteOauthLoginSessionRequest {
    pub(crate) callback_url: String,
    pub(crate) mailbox_session_id: Option<String>,
    #[serde(alias = "generatedMailboxAddress")]
    pub(crate) mailbox_address: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateOauthLoginSessionRequest {
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) display_name: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) email: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) group_name: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) group_bound_proxy_keys: OptionalField<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) group_node_shunt_enabled: OptionalField<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) group_single_account_rotation_enabled: OptionalField<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) note: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) group_note: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) concurrency_limit: OptionalField<i64>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) tag_ids: OptionalField<Vec<i64>>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) is_mother: OptionalField<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) mailbox_session_id: OptionalField<String>,
    #[serde(
        default,
        alias = "generatedMailboxAddress",
        deserialize_with = "deserialize_optional_field"
    )]
    pub(crate) mailbox_address: OptionalField<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateOauthMailboxSessionRequest {
    pub(crate) email_address: Option<String>,
}
