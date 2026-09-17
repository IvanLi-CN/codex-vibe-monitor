#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ParsedUsage {
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    pub(crate) total_tokens: Option<i64>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub(crate) struct ProxyCostBreakdown {
    pub(crate) input: f64,
    pub(crate) cache_write: f64,
    pub(crate) cache_read: f64,
    pub(crate) output: f64,
    pub(crate) reasoning: f64,
}

impl ProxyCostBreakdown {
    pub(crate) fn total(self) -> f64 {
        self.input + self.cache_write + self.cache_read + self.output + self.reasoning
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct RawPayloadMeta {
    pub(crate) path: Option<String>,
    pub(crate) size_bytes: i64,
    pub(crate) truncated: bool,
    pub(crate) truncated_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompactionKind {
    Compact,
    RemoteV2,
}

impl CompactionKind {
    pub(crate) fn as_payload_str(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::RemoteV2 => "remote_v2",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RequestCaptureInfo {
    pub(crate) model: Option<String>,
    pub(crate) sticky_key: Option<String>,
    pub(crate) prompt_cache_key: Option<String>,
    pub(crate) prompt_cache_key_attribution_source: Option<String>,
    pub(crate) contains_encrypted_content: bool,
    pub(crate) image_intent: Option<String>,
    pub(crate) requested_service_tier: Option<String>,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) compaction_request_kind: Option<CompactionKind>,
    pub(crate) is_stream: bool,
    pub(crate) parse_error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ResponseCaptureInfo {
    pub(crate) model: Option<String>,
    pub(crate) contains_encrypted_content: bool,
    pub(crate) usage: ParsedUsage,
    pub(crate) usage_missing_reason: Option<String>,
    pub(crate) service_tier: Option<String>,
    pub(crate) compaction_response_kind: Option<CompactionKind>,
    pub(crate) stream_terminal_event: Option<String>,
    pub(crate) upstream_error_code: Option<String>,
    pub(crate) upstream_error_message: Option<String>,
    pub(crate) upstream_request_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) struct StageTimings {
    pub(crate) t_total_ms: f64,
    pub(crate) t_req_read_ms: f64,
    pub(crate) t_req_parse_ms: f64,
    pub(crate) t_upstream_connect_ms: f64,
    pub(crate) t_upstream_ttfb_ms: f64,
    pub(crate) first_token_ms: Option<f64>,
    pub(crate) t_upstream_stream_ms: f64,
    pub(crate) t_resp_parse_ms: f64,
    pub(crate) t_persist_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProxyCaptureRecord {
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) model: Option<String>,
    pub(crate) usage: ParsedUsage,
    pub(crate) cost: Option<f64>,
    pub(crate) cost_breakdown: Option<ProxyCostBreakdown>,
    pub(crate) cost_estimated: bool,
    pub(crate) price_version: Option<String>,
    pub(crate) status: String,
    pub(crate) error_message: Option<String>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) payload: Option<String>,
    pub(crate) raw_response: String,
    pub(crate) response_body_preview_enabled: bool,
    pub(crate) req_raw: RawPayloadMeta,
    pub(crate) resp_raw: RawPayloadMeta,
    pub(crate) timings: StageTimings,
}

impl ProxyCaptureRecord {
    pub(crate) fn estimated_memory_bytes(&self) -> usize {
        self.invoke_id.capacity()
            + self.occurred_at.capacity()
            + self.model.as_ref().map_or(0, String::capacity)
            + self.payload.as_ref().map_or(0, String::capacity)
            + self.raw_response.capacity()
            + self.price_version.as_ref().map_or(0, String::capacity)
            + self.status.capacity()
            + self.error_message.as_ref().map_or(0, String::capacity)
            + self.failure_kind.as_ref().map_or(0, String::capacity)
            + self.req_raw.path.as_ref().map_or(0, String::capacity)
            + self
                .req_raw
                .truncated_reason
                .as_ref()
                .map_or(0, String::capacity)
            + self.resp_raw.path.as_ref().map_or(0, String::capacity)
            + self
                .resp_raw
                .truncated_reason
                .as_ref()
                .map_or(0, String::capacity)
            + std::mem::size_of::<ParsedUsage>()
            + self
                .cost_breakdown
                .as_ref()
                .map_or(0, |_| std::mem::size_of::<ProxyCostBreakdown>())
            + std::mem::size_of::<Self>()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RequestBodyReadError {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
    pub(crate) failure_kind: &'static str,
    pub(crate) partial_body: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProxyCaptureTarget {
    ChatCompletions,
    Responses,
    ResponsesCompact,
    StandaloneSearch,
    ImageGenerations,
    ImageEdits,
}

impl ProxyCaptureTarget {
    pub(crate) fn endpoint(self) -> &'static str {
        match self {
            Self::ChatCompletions => "/v1/chat/completions",
            Self::Responses => "/v1/responses",
            Self::ResponsesCompact => "/v1/responses/compact",
            Self::StandaloneSearch => "/v1/alpha/search",
            Self::ImageGenerations => "/v1/images/generations",
            Self::ImageEdits => "/v1/images/edits",
        }
    }

    pub(crate) fn allows_fast_mode_rewrite(self) -> bool {
        matches!(self, Self::ChatCompletions | Self::Responses)
    }

    pub(crate) fn should_auto_include_usage(self) -> bool {
        matches!(self, Self::ChatCompletions)
    }

    pub(crate) fn from_endpoint(endpoint: &str) -> Self {
        match endpoint {
            "/v1/chat/completions" => Self::ChatCompletions,
            "/v1/responses/compact" => Self::ResponsesCompact,
            "/v1/responses" => Self::Responses,
            "/v1/alpha/search" => Self::StandaloneSearch,
            "/v1/images/generations" => Self::ImageGenerations,
            "/v1/images/edits" => Self::ImageEdits,
            _ => Self::Responses,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InvocationSourceScope {
    ProxyOnly,
    All,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ProxyUsageBackfillSummary {
    pub(crate) scanned: u64,
    pub(crate) updated: u64,
    pub(crate) skipped_missing_file: u64,
    pub(crate) skipped_without_usage: u64,
    pub(crate) skipped_decode_error: u64,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ProxyCostBackfillSummary {
    pub(crate) scanned: u64,
    pub(crate) updated: u64,
    pub(crate) skipped_unpriced_model: u64,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ProxyPromptCacheKeyBackfillSummary {
    pub(crate) scanned: u64,
    pub(crate) updated: u64,
    pub(crate) skipped_missing_file: u64,
    pub(crate) skipped_invalid_json: u64,
    pub(crate) skipped_missing_key: u64,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ProxyRequestedServiceTierBackfillSummary {
    pub(crate) scanned: u64,
    pub(crate) updated: u64,
    pub(crate) skipped_missing_file: u64,
    pub(crate) skipped_invalid_json: u64,
    pub(crate) skipped_missing_tier: u64,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct InvocationServiceTierBackfillSummary {
    pub(crate) scanned: u64,
    pub(crate) updated: u64,
    pub(crate) skipped_missing_file: u64,
    pub(crate) skipped_missing_tier: u64,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ProxyReasoningEffortBackfillSummary {
    pub(crate) scanned: u64,
    pub(crate) updated: u64,
    pub(crate) skipped_missing_file: u64,
    pub(crate) skipped_invalid_json: u64,
    pub(crate) skipped_missing_effort: u64,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct FailureClassificationBackfillSummary {
    pub(crate) scanned: u64,
    pub(crate) updated: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureClass {
    None,
    ServiceFailure,
    ClientFailure,
    ClientAbort,
}

impl FailureClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            FailureClass::None => FAILURE_CLASS_NONE,
            FailureClass::ServiceFailure => FAILURE_CLASS_SERVICE,
            FailureClass::ClientFailure => FAILURE_CLASS_CLIENT,
            FailureClass::ClientAbort => FAILURE_CLASS_ABORT,
        }
    }

    pub(crate) fn from_db_str(raw: &str) -> Option<Self> {
        match raw {
            FAILURE_CLASS_NONE => Some(FailureClass::None),
            FAILURE_CLASS_SERVICE => Some(FailureClass::ServiceFailure),
            FAILURE_CLASS_CLIENT => Some(FailureClass::ClientFailure),
            FAILURE_CLASS_ABORT => Some(FailureClass::ClientAbort),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FailureClassification {
    pub(crate) failure_kind: Option<String>,
    pub(crate) failure_class: FailureClass,
    pub(crate) is_actionable: bool,
}

#[derive(Debug, FromRow)]
pub(crate) struct ProxyUsageBackfillCandidate {
    pub(crate) id: i64,
    pub(crate) response_raw_path: String,
    pub(crate) payload: Option<String>,
}

#[derive(Debug, FromRow)]
pub(crate) struct ProxyCostBackfillCandidate {
    pub(crate) id: i64,
    pub(crate) model: Option<String>,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    pub(crate) total_tokens: Option<i64>,
    pub(crate) requested_service_tier: Option<String>,
    pub(crate) service_tier: Option<String>,
    pub(crate) snapshot_upstream_account_kind: Option<String>,
    pub(crate) snapshot_upstream_base_url_host: Option<String>,
    pub(crate) live_upstream_base_url_host: Option<String>,
    pub(crate) live_upstream_account_kind: Option<String>,
    pub(crate) live_upstream_account_snapshot_safe: i64,
}

#[derive(Debug, FromRow)]
pub(crate) struct ProxyPromptCacheKeyBackfillCandidate {
    pub(crate) id: i64,
    pub(crate) request_raw_path: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct ProxyRequestedServiceTierBackfillCandidate {
    pub(crate) id: i64,
    pub(crate) request_raw_path: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct ProxyReasoningEffortBackfillCandidate {
    pub(crate) id: i64,
    pub(crate) request_raw_path: String,
}

#[derive(Debug)]
pub(crate) struct ProxyUsageBackfillUpdate {
    pub(crate) id: i64,
    pub(crate) usage: ParsedUsage,
}

#[derive(Debug)]
pub(crate) struct ProxyCostBackfillUpdate {
    pub(crate) id: i64,
    pub(crate) cost: Option<f64>,
    pub(crate) cost_estimated: bool,
    pub(crate) price_version: Option<String>,
    pub(crate) billing_service_tier: Option<String>,
    pub(crate) upstream_account_kind: Option<String>,
    pub(crate) upstream_base_url_host: Option<String>,
}
