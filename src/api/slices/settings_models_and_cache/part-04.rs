#[derive(Debug, FromRow)]
pub(crate) struct PromptCacheConversationAggregateRow {
    pub(crate) prompt_cache_key: String,
    pub(crate) request_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) created_at: String,
    pub(crate) last_activity_at: String,
    #[sqlx(default)]
    pub(crate) cursor_created_at: Option<String>,
    #[sqlx(default)]
    pub(crate) sort_anchor_at: Option<String>,
    #[sqlx(default)]
    pub(crate) last_terminal_at: Option<String>,
    #[sqlx(default)]
    pub(crate) last_in_flight_at: Option<String>,
}

#[derive(Debug, FromRow)]
pub(crate) struct PromptCacheConversationEventRow {
    pub(crate) occurred_at: String,
    pub(crate) status: String,
    pub(crate) error_message: Option<String>,
    pub(crate) downstream_error_message: Option<String>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) failure_class: Option<String>,
    pub(crate) request_tokens: i64,
    pub(crate) prompt_cache_key: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct PromptCacheConversationInvocationPreviewRow {
    pub(crate) prompt_cache_key: String,
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) status: String,
    pub(crate) live_phase: Option<String>,
    pub(crate) failure_class: Option<String>,
    pub(crate) route_mode: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) request_model: Option<String>,
    pub(crate) response_model: Option<String>,
    pub(crate) total_tokens: i64,
    pub(crate) cost: Option<f64>,
    pub(crate) source: Option<String>,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) error_message: Option<String>,
    pub(crate) downstream_status_code: Option<i64>,
    pub(crate) downstream_error_message: Option<String>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) blocked_binding_json: Option<String>,
    pub(crate) is_actionable: Option<i64>,
    pub(crate) proxy_display_name: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) upstream_account_plan_type: Option<String>,
    pub(crate) response_content_encoding: Option<String>,
    pub(crate) request_compression_algorithm: Option<String>,
    pub(crate) transport: Option<String>,
    pub(crate) requested_service_tier: Option<String>,
    pub(crate) service_tier: Option<String>,
    pub(crate) billing_service_tier: Option<String>,
    pub(crate) t_req_read_ms: Option<f64>,
    pub(crate) t_req_parse_ms: Option<f64>,
    pub(crate) t_upstream_connect_ms: Option<f64>,
    pub(crate) t_upstream_ttfb_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) first_token_ms: Option<f64>,
    pub(crate) t_upstream_stream_ms: Option<f64>,
    pub(crate) t_resp_parse_ms: Option<f64>,
    pub(crate) t_persist_ms: Option<f64>,
    pub(crate) t_total_ms: Option<f64>,
    pub(crate) endpoint: Option<String>,
    pub(crate) compaction_request_kind: Option<String>,
    pub(crate) compaction_response_kind: Option<String>,
    pub(crate) image_intent: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct UpstreamAccountInvocationPreviewRow {
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    pub(crate) prompt_cache_key: Option<String>,
    pub(crate) occurred_at: String,
    pub(crate) conversation_created_at: Option<String>,
    pub(crate) status: String,
    pub(crate) live_phase: Option<String>,
    pub(crate) failure_class: Option<String>,
    pub(crate) route_mode: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) request_model: Option<String>,
    pub(crate) response_model: Option<String>,
    pub(crate) total_tokens: i64,
    pub(crate) cost: Option<f64>,
    pub(crate) cost_input: Option<f64>,
    pub(crate) cost_cache_write: Option<f64>,
    pub(crate) cost_cache_read: Option<f64>,
    pub(crate) cost_output: Option<f64>,
    pub(crate) cost_reasoning: Option<f64>,
    pub(crate) source: Option<String>,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) error_message: Option<String>,
    pub(crate) downstream_status_code: Option<i64>,
    pub(crate) downstream_error_message: Option<String>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) is_actionable: Option<i64>,
    pub(crate) proxy_display_name: Option<String>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) upstream_account_plan_type: Option<String>,
    pub(crate) response_content_encoding: Option<String>,
    pub(crate) request_compression_algorithm: Option<String>,
    pub(crate) transport: Option<String>,
    pub(crate) requested_service_tier: Option<String>,
    pub(crate) service_tier: Option<String>,
    pub(crate) billing_service_tier: Option<String>,
    pub(crate) t_req_read_ms: Option<f64>,
    pub(crate) t_req_parse_ms: Option<f64>,
    pub(crate) t_upstream_connect_ms: Option<f64>,
    pub(crate) t_upstream_ttfb_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) first_token_ms: Option<f64>,
    pub(crate) t_upstream_stream_ms: Option<f64>,
    pub(crate) t_resp_parse_ms: Option<f64>,
    pub(crate) t_persist_ms: Option<f64>,
    pub(crate) t_total_ms: Option<f64>,
    pub(crate) endpoint: Option<String>,
    pub(crate) compaction_request_kind: Option<String>,
    pub(crate) compaction_response_kind: Option<String>,
    pub(crate) image_intent: Option<String>,
}

#[derive(Debug, FromRow)]
pub(crate) struct PromptCacheConversationUpstreamAccountSummaryRow {
    pub(crate) prompt_cache_key: String,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) request_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) last_activity_at: String,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct PromptCacheConversationEncryptedOwnerSummaryRow {
    pub(crate) prompt_cache_key: String,
    pub(crate) owner_upstream_account_id: i64,
    pub(crate) owner_upstream_account_name: Option<String>,
    pub(crate) owner_group_name: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct PromptCacheConversationManualBindingSummaryRow {
    pub(crate) prompt_cache_key: String,
    pub(crate) binding_kind: String,
    pub(crate) group_name: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
}

#[derive(Debug, FromRow)]
pub(crate) struct ParallelWorkExactInvocationRow {
    pub(crate) occurred_at: String,
    pub(crate) prompt_cache_key: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct ParallelWorkConversationSpanRow {
    pub(crate) conversation_id: String,
    pub(crate) first_occurred_at: String,
    pub(crate) last_occurred_at: String,
    pub(crate) request_count: i64,
}

#[derive(Debug, FromRow)]
pub(crate) struct ParallelWorkDayRollupRow {
    pub(crate) bucket_start_epoch: i64,
    pub(crate) prompt_cache_key: String,
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListQuery {
    pub(crate) limit: Option<i64>,
    pub(crate) page: Option<i64>,
    pub(crate) page_size: Option<i64>,
    pub(crate) snapshot_id: Option<i64>,
    pub(crate) anchor_id: Option<String>,
    pub(crate) sort_by: Option<String>,
    pub(crate) sort_order: Option<String>,
    #[allow(dead_code)]
    pub(crate) range_preset: Option<String>,
    pub(crate) from: Option<String>,
    pub(crate) to: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) models: Option<String>,
    pub(crate) model_target: Option<String>,
    pub(crate) model_rerouted: Option<String>,
    pub(crate) status: Option<String>,
    // Kept for compatibility so stale /records URLs with `?proxy=...` deserialize cleanly,
    // but records queries intentionally ignore this field.
    #[allow(dead_code)]
    pub(crate) proxy: Option<String>,
    pub(crate) endpoint: Option<String>,
    pub(crate) invoke_id: Option<String>,
    pub(crate) attempt_id: Option<String>,
    // Kept for compatibility so stale /records URLs with `?requestId=...` still resolve.
    // Owner-facing records surfaces should use `invokeId` instead.
    pub(crate) request_id: Option<String>,
    pub(crate) failure_class: Option<String>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) prompt_cache_key: Option<String>,
    pub(crate) sticky_key: Option<String>,
    pub(crate) upstream_scope: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) proxy_display_name: Option<String>,
    pub(crate) transport: Option<String>,
    pub(crate) service_tier: Option<String>,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) reasoning_efforts: Option<String>,
    pub(crate) requester_ip: Option<String>,
    pub(crate) keyword: Option<String>,
    pub(crate) min_total_tokens: Option<i64>,
    pub(crate) max_total_tokens: Option<i64>,
    pub(crate) min_total_ms: Option<f64>,
    pub(crate) max_total_ms: Option<f64>,
    pub(crate) suggest_field: Option<String>,
    pub(crate) suggest_query: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocateInvocationQuery {
    pub(crate) invoke_id: Option<String>,
    pub(crate) request_id: Option<String>,
    pub(crate) attempt_id: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) page_size: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationsQuery {
    pub(crate) limit: Option<i64>,
    pub(crate) activity_hours: Option<i64>,
    pub(crate) activity_minutes: Option<i64>,
    pub(crate) page_size: Option<i64>,
    pub(crate) cursor: Option<String>,
    pub(crate) snapshot_at: Option<String>,
    pub(crate) detail: Option<String>,
    pub(crate) recent_invocation_limit: Option<i64>,
    pub(crate) blocked_binding_upstream_account_id: Option<i64>,
    pub(crate) blocked_binding_constraint_source: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SummaryQuery {
    pub(crate) window: Option<String>,
    pub(crate) limit: Option<i64>,
    pub(crate) time_zone: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TimeseriesQuery {
    #[serde(default = "default_range")]
    pub(crate) range: String,
    pub(crate) bucket: Option<String>,
    #[allow(dead_code)]
    pub(crate) settlement_hour: Option<u8>,
    pub(crate) time_zone: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ParallelWorkStatsQuery {
    #[serde(default = "default_range")]
    pub(crate) range: String,
    pub(crate) bucket: Option<String>,
    pub(crate) time_zone: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountActivityQuery {
    #[serde(default = "default_range")]
    pub(crate) range: String,
    pub(crate) recent_limit: Option<i64>,
    pub(crate) time_zone: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardActivityQuery {
    #[serde(default = "default_range")]
    pub(crate) range: String,
    pub(crate) recent_limit: Option<i64>,
    pub(crate) time_zone: Option<String>,
    #[serde(default)]
    pub(crate) include_accounts: bool,
    pub(crate) include_recent: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardActivityRecentQuery {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) snapshot_id: i64,
    pub(crate) recent_limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardNetworkTimeseriesQuery {
    #[serde(default = "default_range")]
    pub(crate) range: String,
    pub(crate) time_zone: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardRecentNetworkWindowQuery {}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PerfQuery {
    #[serde(default = "default_range")]
    pub(crate) range: String,
    pub(crate) time_zone: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PerfStatsResponse {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) source: String,
    pub(crate) stages: Vec<PerfStageStats>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountActivityResponse {
    pub(crate) range: String,
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) routing_state_version: Option<crate::upstream_accounts::RoutingStateVersion>,
    pub(crate) accounts: Vec<UpstreamAccountActivityAccountResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardActivityRateWindowResponse {
    pub(crate) start: String,
    pub(crate) end: String,
    pub(crate) window_minutes: i64,
    pub(crate) mode: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelPerformanceMetricsResponse {
    pub(crate) tokens_per_minute: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) streaming_response_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) avg_response_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) avg_first_response_byte_total_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) avg_first_token_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) wall_clock_usage_duration_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cumulative_usage_duration_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) parallelism: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelPerformanceModelResponse {
    pub(crate) model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning_effort: Option<String>,
    #[serde(flatten)]
    pub(crate) metrics: ModelPerformanceMetricsResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelPerformanceResponse {
    pub(crate) available: bool,
    pub(crate) total: ModelPerformanceMetricsResponse,
    pub(crate) models: Vec<ModelPerformanceModelResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardActivitySummaryResponse {
    pub(crate) stats: StatsResponse,
    pub(crate) tokens_per_minute: Option<f64>,
    pub(crate) spend_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current_first_response_byte_total_avg_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current_first_token_avg_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current_avg_total_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current_avg_response_ms: Option<f64>,
    pub(crate) model_performance: ModelPerformanceResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardActivityResponse {
    pub(crate) range: String,
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) snapshot_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) routing_state_version: Option<crate::upstream_accounts::RoutingStateVersion>,
    #[serde(skip)]
    pub(crate) terminal_sequence: u64,
    pub(crate) live_revision: u64,
    pub(crate) rate_window: DashboardActivityRateWindowResponse,
    pub(crate) summary: DashboardActivitySummaryResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) network_live_bucket: Option<DashboardNetworkTimeseriesPointResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) network_realtime_rate: Option<DashboardNetworkRealtimeRateResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) accounts: Option<Vec<DashboardActivityAccountResponse>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardActivityRecentAccountResponse {
    pub(crate) account_key: String,
    pub(crate) recent_invocations: Vec<PromptCacheConversationInvocationPreviewResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardActivityRecentResponse {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) snapshot_id: i64,
    pub(crate) accounts: Vec<DashboardActivityRecentAccountResponse>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardNetworkTimeseriesPointResponse {
    pub(crate) bucket_start: String,
    pub(crate) bucket_end: String,
    pub(crate) upload_bytes_per_second: f64,
    pub(crate) download_bytes_per_second: f64,
    pub(crate) upload_bytes: i64,
    pub(crate) download_bytes: i64,
    pub(crate) is_live_bucket: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardNetworkRealtimeRateResponse {
    pub(crate) sample_start: String,
    pub(crate) sample_end: String,
    pub(crate) sample_seconds: i64,
    pub(crate) upload_bytes_per_second: f64,
    pub(crate) download_bytes_per_second: f64,
    pub(crate) upload_bytes: i64,
    pub(crate) download_bytes: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardNetworkTimeseriesResponse {
    pub(crate) range: String,
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) snapshot_id: i64,
    pub(crate) bucket_seconds: i64,
    pub(crate) points: Vec<DashboardNetworkTimeseriesPointResponse>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardRecentNetworkWindowPointResponse {
    pub(crate) sample_start: String,
    pub(crate) sample_end: String,
    pub(crate) upload_bytes_per_second: f64,
    pub(crate) download_bytes_per_second: f64,
    pub(crate) upload_bytes: i64,
    pub(crate) download_bytes: i64,
    pub(crate) is_available: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardRecentNetworkWindowResponse {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) window_seconds: i64,
    pub(crate) sample_seconds: i64,
    pub(crate) is_warming_up: bool,
    pub(crate) points: Vec<DashboardRecentNetworkWindowPointResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardActivityAccountResponse {
    pub(crate) account_key: String,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) display_name: String,
    pub(crate) is_unassigned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) latest_conversation_created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_invocation_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) group_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) plan_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) display_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) enable_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) work_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) health_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sync_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_action_reason_message: Option<String>,
    pub(crate) request_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) non_success_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) success_tokens: i64,
    pub(crate) non_success_tokens: i64,
    pub(crate) failure_tokens: i64,
    pub(crate) failure_cost: f64,
    #[serde(skip)]
    pub(crate) non_success_cost: f64,
    pub(crate) total_cost: f64,
    pub(crate) usage_breakdown: UsageBreakdownResponse,
    pub(crate) model_performance: ModelPerformanceResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cache_hit_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tokens_per_minute: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) spend_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) first_byte_avg_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) first_response_byte_total_avg_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) first_token_avg_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) avg_total_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current_first_response_byte_total_avg_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current_first_token_avg_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current_avg_total_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current_avg_response_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) in_progress_invocation_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) in_progress_phase_counts: Option<InvocationPhaseCountsResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) retry_invocation_count: Option<i64>,
    pub(crate) upload_bytes_per_second: f64,
    pub(crate) download_bytes_per_second: f64,
    #[serde(skip)]
    pub(crate) in_progress_wait_sum_ms: f64,
    #[serde(skip)]
    pub(crate) in_progress_wait_sample_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) effective_routing_rule: Option<crate::upstream_accounts::EffectiveRoutingRule>,
    pub(crate) recent_invocations: Vec<PromptCacheConversationInvocationPreviewResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountActivityAccountResponse {
    pub(crate) upstream_account_id: i64,
    pub(crate) display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) latest_conversation_created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_invocation_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) group_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) plan_type: Option<String>,
    pub(crate) enabled: bool,
    pub(crate) display_status: String,
    pub(crate) enable_status: String,
    pub(crate) work_status: String,
    pub(crate) health_status: String,
    pub(crate) sync_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_action_reason_message: Option<String>,
    pub(crate) request_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) non_success_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) success_tokens: i64,
    pub(crate) non_success_tokens: i64,
    pub(crate) failure_tokens: i64,
    pub(crate) failure_cost: f64,
    pub(crate) total_cost: f64,
    pub(crate) usage_breakdown: UsageBreakdownResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cache_hit_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tokens_per_minute: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) spend_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) first_byte_avg_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) first_response_byte_total_avg_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) first_token_avg_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) avg_total_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current_first_response_byte_total_avg_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current_first_token_avg_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current_avg_total_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current_avg_response_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) in_progress_invocation_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) in_progress_phase_counts: Option<InvocationPhaseCountsResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) retry_invocation_count: Option<i64>,
    pub(crate) upload_bytes_per_second: f64,
    pub(crate) download_bytes_per_second: f64,
    pub(crate) effective_routing_rule: crate::upstream_accounts::EffectiveRoutingRule,
    pub(crate) recent_invocations: Vec<PromptCacheConversationInvocationPreviewResponse>,
}
