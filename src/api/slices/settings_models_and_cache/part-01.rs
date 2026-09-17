#[derive(Debug, Clone, Default)]
pub(crate) struct RequestCompressionDerivedFields {
    pub(crate) logical_body_bytes: Option<i64>,
    pub(crate) transmitted_body_bytes: Option<i64>,
    pub(crate) saved_bytes: Option<i64>,
    pub(crate) ratio_pct: Option<f64>,
    pub(crate) approx_upload_bytes: Option<i64>,
    pub(crate) approx_download_bytes: Option<i64>,
}

pub(crate) fn derive_request_compression_fields(
    logical_body_bytes: Option<i64>,
    transmitted_body_bytes: Option<i64>,
    request_header_bytes_approx: Option<i64>,
    response_body_bytes: Option<i64>,
    response_header_bytes_approx: Option<i64>,
    transmission_complete: bool,
) -> RequestCompressionDerivedFields {
    let logical_body_bytes = logical_body_bytes.filter(|value| *value >= 0);
    let transmitted_body_bytes = transmitted_body_bytes.filter(|value| *value >= 0);
    let request_header_bytes_approx = request_header_bytes_approx.filter(|value| *value >= 0);
    let response_body_bytes = response_body_bytes.filter(|value| *value >= 0);
    let response_header_bytes_approx = response_header_bytes_approx.filter(|value| *value >= 0);
    let approx_upload_bytes = match (request_header_bytes_approx, transmitted_body_bytes) {
        (Some(headers), Some(body)) => Some(headers.saturating_add(body)),
        (Some(headers), None) => Some(headers),
        (None, Some(body)) => Some(body),
        (None, None) => None,
    };
    let approx_download_bytes = match (response_header_bytes_approx, response_body_bytes) {
        (Some(headers), Some(body)) => Some(headers.saturating_add(body)),
        (Some(headers), None) => Some(headers),
        (None, Some(body)) => Some(body),
        (None, None) => None,
    };
    let (saved_bytes, ratio_pct) = if transmission_complete
        && let (Some(logical), Some(transmitted)) = (logical_body_bytes, transmitted_body_bytes)
    {
        let saved_bytes = logical.saturating_sub(transmitted);
        let ratio_pct = if logical == 0 {
            Some(if transmitted == 0 { 0.0 } else { 100.0 })
        } else {
            Some(((transmitted - logical) as f64 / logical as f64) * 100.0)
        };
        (Some(saved_bytes), ratio_pct)
    } else {
        (None, None)
    };

    RequestCompressionDerivedFields {
        logical_body_bytes,
        transmitted_body_bytes,
        saved_bytes,
        ratio_pct,
        approx_upload_bytes,
        approx_download_bytes,
    }
}

pub(crate) fn hydrate_pool_attempt_request_compression_fields(
    records: &mut [ApiPoolUpstreamRequestAttempt],
) {
    for record in records {
        let derived = derive_request_compression_fields(
            record.upstream_request_logical_body_bytes,
            record.upstream_request_transmitted_body_bytes,
            record.upstream_request_header_bytes_approx,
            record.upstream_response_body_bytes,
            record.upstream_response_header_bytes_approx,
            record.http_status.is_some()
                || record
                    .status
                    .eq_ignore_ascii_case(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS),
        );
        record.logical_body_bytes = derived.logical_body_bytes;
        record.transmitted_body_bytes = derived.transmitted_body_bytes;
        record.saved_bytes = derived.saved_bytes;
        record.ratio_pct = derived.ratio_pct;
        record.approx_upload_bytes = derived.approx_upload_bytes;
        record.approx_download_bytes = derived.approx_download_bytes;
    }
}

pub(crate) fn hydrate_pool_attempt_routing_selection_audits(
    records: &mut [ApiPoolUpstreamRequestAttempt],
) {
    for record in records {
        record.routing_selection_audit = record
            .routing_selection_audit_json
            .as_deref()
            .and_then(|value| serde_json::from_str(value).ok());
    }
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApiPoolUpstreamRequestAttempt {
    #[serde(skip_serializing)]
    pub(crate) id: i64,
    pub(crate) attempt_id: String,
    pub(crate) invoke_id: String,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) occurred_at: String,
    pub(crate) endpoint: String,
    #[sqlx(default)]
    pub(crate) sticky_key: Option<String>,
    #[sqlx(default)]
    pub(crate) routing_source: Option<String>,
    #[sqlx(default)]
    #[serde(skip_serializing)]
    pub(crate) routing_selection_audit_json: Option<String>,
    #[sqlx(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) routing_selection_audit: Option<PoolRoutingSelectionAudit>,
    #[sqlx(default)]
    pub(crate) upstream_account_id: Option<i64>,
    #[sqlx(default)]
    pub(crate) upstream_account_name: Option<String>,
    #[sqlx(default)]
    pub(crate) model: Option<String>,
    #[sqlx(default)]
    pub(crate) request_model: Option<String>,
    #[sqlx(default)]
    pub(crate) upstream_request_model: Option<String>,
    #[sqlx(default)]
    pub(crate) model_mapping_pattern: Option<String>,
    #[sqlx(default)]
    pub(crate) response_model: Option<String>,
    #[sqlx(default)]
    pub(crate) compaction_request_kind: Option<String>,
    #[sqlx(default)]
    pub(crate) compaction_response_kind: Option<String>,
    #[sqlx(default)]
    pub(crate) image_intent: Option<String>,
    #[sqlx(default)]
    pub(crate) upstream_route_key: Option<String>,
    #[sqlx(default)]
    pub(crate) proxy_binding_key_snapshot: Option<String>,
    pub(crate) attempt_index: i64,
    pub(crate) distinct_account_index: i64,
    pub(crate) same_account_retry_index: i64,
    #[sqlx(default)]
    pub(crate) requester_ip: Option<String>,
    #[sqlx(default)]
    #[serde(serialize_with = "serialize_opt_local_or_utc_to_utc_iso")]
    pub(crate) started_at: Option<String>,
    #[sqlx(default)]
    #[serde(serialize_with = "serialize_opt_local_or_utc_to_utc_iso")]
    pub(crate) finished_at: Option<String>,
    pub(crate) status: String,
    pub(crate) phase: String,
    #[sqlx(default)]
    pub(crate) http_status: Option<i64>,
    #[sqlx(default)]
    pub(crate) downstream_http_status: Option<i64>,
    #[sqlx(default)]
    pub(crate) failure_kind: Option<String>,
    #[sqlx(default)]
    pub(crate) error_message: Option<String>,
    #[sqlx(default)]
    pub(crate) downstream_error_message: Option<String>,
    #[sqlx(default)]
    #[serde(serialize_with = "serialize_opt_finite_nonnegative_timing")]
    pub(crate) connect_latency_ms: Option<f64>,
    #[sqlx(default)]
    #[serde(serialize_with = "serialize_opt_finite_nonnegative_timing")]
    pub(crate) first_token_ms: Option<f64>,
    #[sqlx(default)]
    #[serde(serialize_with = "serialize_opt_finite_nonnegative_timing")]
    pub(crate) first_byte_latency_ms: Option<f64>,
    #[sqlx(default)]
    #[serde(serialize_with = "serialize_opt_finite_positive_timing")]
    pub(crate) stream_latency_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) upstream_request_id: Option<String>,
    #[sqlx(default)]
    pub(crate) downstream_request_content_encoding: Option<String>,
    #[sqlx(default)]
    pub(crate) upstream_request_compression_algorithm: Option<String>,
    #[sqlx(default)]
    pub(crate) upstream_request_compression_mode: Option<String>,
    #[sqlx(default)]
    #[serde(skip_serializing)]
    pub(crate) upstream_request_logical_body_bytes: Option<i64>,
    #[sqlx(default)]
    #[serde(skip_serializing)]
    pub(crate) upstream_request_transmitted_body_bytes: Option<i64>,
    #[sqlx(default)]
    #[serde(skip_serializing)]
    pub(crate) upstream_request_header_bytes_approx: Option<i64>,
    #[sqlx(default)]
    #[serde(skip_serializing)]
    pub(crate) upstream_response_body_bytes: Option<i64>,
    #[sqlx(default)]
    #[serde(skip_serializing)]
    pub(crate) upstream_response_header_bytes_approx: Option<i64>,
    #[sqlx(default)]
    pub(crate) logical_body_bytes: Option<i64>,
    #[sqlx(default)]
    pub(crate) transmitted_body_bytes: Option<i64>,
    #[sqlx(default)]
    pub(crate) saved_bytes: Option<i64>,
    #[sqlx(default)]
    pub(crate) ratio_pct: Option<f64>,
    #[sqlx(default)]
    pub(crate) approx_upload_bytes: Option<i64>,
    #[sqlx(default)]
    pub(crate) approx_download_bytes: Option<i64>,
    #[sqlx(default)]
    pub(crate) compact_support_status: Option<String>,
    #[sqlx(default)]
    pub(crate) compact_support_reason: Option<String>,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) created_at: String,
    #[sqlx(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) invocation_record: Option<ApiInvocation>,
    #[sqlx(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) workflow_entry: Option<InvocationWorkflowTimelineEntry>,
}

pub(crate) fn sanitize_pool_attempt_timing_fields(records: &mut [ApiPoolUpstreamRequestAttempt]) {
    for record in records {
        record.connect_latency_ms = record
            .connect_latency_ms
            .filter(|value| value.is_finite() && *value >= 0.0);
        record.first_token_ms = record
            .first_token_ms
            .filter(|value| value.is_finite() && *value >= 0.0);
        record.first_byte_latency_ms = record
            .first_byte_latency_ms
            .filter(|value| value.is_finite() && *value >= 0.0);
        record.stream_latency_ms = record
            .stream_latency_ms
            .filter(|value| value.is_finite() && *value > 0.0);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StatsResponse {
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_cost: f64,
    pub(crate) total_tokens: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) usage_breakdown: Option<UsageBreakdownResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) in_progress_conversation_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) in_progress_retry_conversation_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) in_progress_avg_wait_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) in_progress_phase_counts: Option<InvocationPhaseCountsResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) non_success_cost: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) non_success_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) maintenance: Option<StatsMaintenanceResponse>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageCostBreakdownResponse {
    pub(crate) input: f64,
    pub(crate) cache_write: f64,
    pub(crate) cache_read: f64,
    pub(crate) output: f64,
    pub(crate) reasoning: f64,
    pub(crate) unknown: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageBreakdownModelResponse {
    pub(crate) model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) cache_write_tokens: i64,
    pub(crate) cache_read_tokens: i64,
    pub(crate) output_tokens: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) costs: Option<UsageCostBreakdownResponse>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageBreakdownResponse {
    pub(crate) cache_write_tokens: i64,
    pub(crate) cache_read_tokens: i64,
    pub(crate) output_tokens: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) costs: Option<UsageCostBreakdownResponse>,
    pub(crate) models: Vec<UsageBreakdownModelResponse>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationPhaseCountsResponse {
    pub(crate) queued: i64,
    pub(crate) requesting: i64,
    pub(crate) responding: i64,
}

impl InvocationPhaseCountsResponse {
    pub(crate) fn increment_phase_name(&mut self, phase: Option<&str>) {
        match normalized_runtime_text(phase).as_str() {
            "requesting" => self.requesting += 1,
            "responding" => self.responding += 1,
            _ => self.queued += 1,
        }
    }

    pub(crate) fn decrement_phase_name(&mut self, phase: Option<&str>) {
        match normalized_runtime_text(phase).as_str() {
            "requesting" => self.requesting = self.requesting.saturating_sub(1),
            "responding" => self.responding = self.responding.saturating_sub(1),
            _ => self.queued = self.queued.saturating_sub(1),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StatsMaintenanceResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) raw_compression_backlog: Option<RawCompressionBacklogResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) startup_backfill: Option<StartupBackfillMaintenanceResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) historical_rollup_backfill: Option<HistoricalRollupBackfillMaintenanceResponse>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RawCompressionBacklogResponse {
    pub(crate) oldest_uncompressed_age_secs: u64,
    pub(crate) uncompressed_count: u64,
    pub(crate) uncompressed_bytes: u64,
    pub(crate) alert_level: RawCompressionAlertLevel,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StartupBackfillMaintenanceResponse {
    pub(crate) upstream_activity_archive_pending_accounts: u64,
    pub(crate) zero_update_streak: u32,
    #[serde(serialize_with = "serialize_opt_local_or_utc_to_utc_iso")]
    pub(crate) next_run_after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoricalRollupBackfillMaintenanceResponse {
    pub(crate) pending_buckets: u64,
    pub(crate) legacy_archive_pending: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_materialized_hour: Option<String>,
    pub(crate) alert_level: HistoricalRollupBackfillAlertLevel,
}

#[derive(Debug, FromRow)]
pub(crate) struct StatsRow {
    pub(crate) total_count: i64,
    pub(crate) success_count: Option<i64>,
    pub(crate) failure_count: Option<i64>,
    pub(crate) total_cost: f64,
    pub(crate) total_tokens: i64,
    pub(crate) non_success_cost: f64,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct StatsTotals {
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_cost: f64,
    pub(crate) total_tokens: i64,
    pub(crate) non_success_cost: f64,
}

impl StatsTotals {
    pub(crate) fn add(self, other: StatsTotals) -> StatsTotals {
        StatsTotals {
            total_count: self.total_count + other.total_count,
            success_count: self.success_count + other.success_count,
            failure_count: self.failure_count + other.failure_count,
            total_cost: self.total_cost + other.total_cost,
            total_tokens: self.total_tokens + other.total_tokens,
            non_success_cost: self.non_success_cost + other.non_success_cost,
        }
    }

    pub(crate) fn into_response(self) -> StatsResponse {
        StatsResponse {
            total_count: self.total_count,
            success_count: self.success_count,
            failure_count: self.failure_count,
            total_cost: self.total_cost,
            total_tokens: self.total_tokens,
            usage_breakdown: None,
            in_progress_conversation_count: None,
            in_progress_retry_conversation_count: None,
            in_progress_avg_wait_ms: None,
            in_progress_phase_counts: None,
            non_success_cost: None,
            non_success_tokens: None,
            maintenance: None,
        }
    }
}

impl From<StatsRow> for StatsTotals {
    fn from(value: StatsRow) -> Self {
        Self {
            total_count: value.total_count,
            success_count: value.success_count.unwrap_or(0),
            failure_count: value.failure_count.unwrap_or(0),
            total_cost: value.total_cost,
            total_tokens: value.total_tokens,
            non_success_cost: value.non_success_cost,
        }
    }
}

impl From<StatsRow> for StatsResponse {
    fn from(value: StatsRow) -> Self {
        StatsTotals::from(value).into_response()
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TimeseriesResponse {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) bucket_seconds: i64,
    pub(crate) snapshot_id: i64,
    pub(crate) effective_bucket: String,
    pub(crate) available_buckets: Vec<String>,
    pub(crate) bucket_limited_to_daily: bool,
    pub(crate) points: Vec<TimeseriesPoint>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ParallelWorkStatsResponse {
    pub(crate) current: ParallelWorkWindowResponse,
    pub(crate) minute7d: ParallelWorkWindowResponse,
    pub(crate) hour30d: ParallelWorkWindowResponse,
    pub(crate) day_all: ParallelWorkWindowResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ParallelWorkWindowResponse {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) bucket_seconds: i64,
    pub(crate) complete_bucket_count: i64,
    pub(crate) active_bucket_count: i64,
    pub(crate) active_minute_count: Option<i64>,
    pub(crate) min_count: Option<i64>,
    pub(crate) max_count: Option<i64>,
    pub(crate) avg_count: Option<f64>,
    pub(crate) effective_time_zone: String,
    pub(crate) time_zone_fallback: bool,
    pub(crate) points: Vec<ParallelWorkPoint>,
    pub(crate) conversations: Vec<ParallelWorkConversation>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ParallelWorkPoint {
    pub(crate) bucket_start: String,
    pub(crate) bucket_end: String,
    pub(crate) parallel_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ParallelWorkConversation {
    pub(crate) conversation_id: String,
    pub(crate) start: String,
    pub(crate) end: String,
    pub(crate) request_count: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct TimeseriesBucketSelection {
    pub(crate) bucket_seconds: i64,
    pub(crate) effective_bucket: String,
    pub(crate) available_buckets: Vec<String>,
    pub(crate) bucket_limited_to_daily: bool,
}

pub(crate) fn resolve_timeseries_bucket_selection(
    params: &TimeseriesQuery,
    range_window: &RangeWindow,
    invocation_max_days: u64,
) -> Result<TimeseriesBucketSelection, ApiError> {
    let mut bucket_seconds = if let Some(spec) = params.bucket.as_deref() {
        bucket_seconds_from_spec(spec)
            .ok_or_else(|| anyhow!("unsupported bucket specification: {spec}"))?
    } else {
        default_bucket_seconds(range_window.duration)
    };

    if bucket_seconds <= 0 {
        return Err(ApiError::bad_request(anyhow!(
            "bucket seconds must be positive"
        )));
    }

    let range_seconds = range_window.duration.num_seconds();
    if range_seconds / bucket_seconds > 10_000 {
        // avoid accidentally returning extremely large payloads
        bucket_seconds = range_seconds / 10_000;
    }

    let subhour_supported = range_window.start >= shanghai_retention_cutoff(invocation_max_days);
    let bucket_limited_to_daily = false;
    let effective_bucket_seconds = if bucket_seconds < 3_600 && !subhour_supported {
        3_600
    } else {
        bucket_seconds
    };
    let effective_bucket = bucket_spec_from_seconds(effective_bucket_seconds)
        .map(str::to_string)
        .unwrap_or_else(|| format!("{effective_bucket_seconds}s"));

    Ok(TimeseriesBucketSelection {
        bucket_seconds: effective_bucket_seconds,
        effective_bucket,
        available_buckets: available_timeseries_bucket_specs(subhour_supported),
        bucket_limited_to_daily,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TimeseriesPoint {
    pub(crate) bucket_start: String,
    pub(crate) bucket_end: String,
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) in_flight_count: i64,
    pub(crate) in_flight_phase_counts: InvocationPhaseCountsResponse,
    pub(crate) total_tokens: i64,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    pub(crate) total_cost: f64,
    pub(crate) non_success_cost: f64,
    pub(crate) avg_total_ms: Option<f64>,
    pub(crate) total_latency_sample_count: i64,
    pub(crate) first_byte_sample_count: i64,
    pub(crate) first_byte_avg_ms: Option<f64>,
    pub(crate) first_byte_p95_ms: Option<f64>,
    pub(crate) first_response_byte_total_sample_count: i64,
    pub(crate) first_response_byte_total_avg_ms: Option<f64>,
    pub(crate) first_response_byte_total_p95_ms: Option<f64>,
    pub(crate) first_token_sample_count: i64,
    pub(crate) first_token_avg_ms: Option<f64>,
    pub(crate) first_token_p95_ms: Option<f64>,
}

#[derive(Debug, FromRow)]
pub(crate) struct UpstreamAccountUsageHourlyRollupRecord {
    pub(crate) bucket_start_epoch: i64,
    pub(crate) request_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) cache_input_tokens: i64,
    pub(crate) reasoning_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) non_success_cost: f64,
}

#[derive(Debug, FromRow)]
pub(crate) struct UpstreamAccountUsageBreakdownHourlyRollupRecord {
    pub(crate) bucket_start_epoch: i64,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) model: String,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) request_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) cache_write_tokens: i64,
    pub(crate) cache_read_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) cost_input: f64,
    pub(crate) cost_cache_write: f64,
    pub(crate) cost_cache_read: f64,
    pub(crate) cost_output: f64,
    pub(crate) cost_reasoning: f64,
    pub(crate) cost_unknown: f64,
    pub(crate) has_cost: i64,
    pub(crate) performance_total_tokens: i64,
    pub(crate) performance_stream_output_tokens: i64,
    pub(crate) performance_stream_duration_ms: f64,
    pub(crate) performance_response_sample_count: i64,
    pub(crate) performance_response_sum_ms: f64,
    pub(crate) performance_first_byte_sample_count: i64,
    pub(crate) performance_first_byte_sum_ms: f64,
    pub(crate) performance_first_token_sample_count: i64,
    pub(crate) performance_first_token_sum_ms: f64,
    pub(crate) performance_usage_duration_sample_count: i64,
    pub(crate) performance_usage_duration_sum_ms: f64,
}

#[derive(Debug, FromRow)]
pub(crate) struct UpstreamAccountStatsRollupRecord {
    pub(crate) bucket_start_epoch: i64,
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) in_flight_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) cache_input_tokens: i64,
    pub(crate) reasoning_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) non_success_cost: f64,
    pub(crate) total_latency_sample_count: i64,
    pub(crate) total_latency_sum_ms: f64,
    pub(crate) first_byte_sample_count: i64,
    pub(crate) first_byte_sum_ms: f64,
    pub(crate) first_byte_max_ms: f64,
    pub(crate) first_byte_histogram: String,
    pub(crate) first_response_byte_total_sample_count: i64,
    pub(crate) first_response_byte_total_sum_ms: f64,
    pub(crate) first_response_byte_total_max_ms: f64,
    pub(crate) first_response_byte_total_histogram: String,
    pub(crate) first_token_sample_count: i64,
    pub(crate) first_token_sum_ms: f64,
    pub(crate) first_token_max_ms: f64,
    pub(crate) first_token_histogram: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QuotaSnapshotResponse {
    #[serde(serialize_with = "serialize_local_or_utc_to_utc_iso")]
    pub(crate) captured_at: String,
    pub(crate) amount_limit: Option<f64>,
    pub(crate) used_amount: Option<f64>,
    pub(crate) remaining_amount: Option<f64>,
    pub(crate) period: Option<String>,
    #[serde(serialize_with = "serialize_opt_local_or_utc_to_utc_iso")]
    pub(crate) period_reset_time: Option<String>,
    #[serde(serialize_with = "serialize_opt_local_or_utc_to_utc_iso")]
    pub(crate) expire_time: Option<String>,
    pub(crate) is_active: bool,
    pub(crate) total_cost: f64,
    pub(crate) total_requests: i64,
    pub(crate) total_tokens: i64,
    #[serde(serialize_with = "serialize_opt_local_or_utc_to_utc_iso")]
    pub(crate) last_request_time: Option<String>,
    pub(crate) billing_type: Option<String>,
    pub(crate) remaining_count: Option<i64>,
    pub(crate) used_count: Option<i64>,
    pub(crate) sub_type_name: Option<String>,
}

#[derive(Debug, FromRow)]
pub(crate) struct QuotaSnapshotRow {
    pub(crate) captured_at: String,
    pub(crate) amount_limit: Option<f64>,
    pub(crate) used_amount: Option<f64>,
    pub(crate) remaining_amount: Option<f64>,
    pub(crate) period: Option<String>,
    pub(crate) period_reset_time: Option<String>,
    pub(crate) expire_time: Option<String>,
    pub(crate) is_active: Option<i64>,
    pub(crate) total_cost: f64,
    pub(crate) total_requests: i64,
    pub(crate) total_tokens: i64,
    pub(crate) last_request_time: Option<String>,
    pub(crate) billing_type: Option<String>,
    pub(crate) remaining_count: Option<i64>,
    pub(crate) used_count: Option<i64>,
    pub(crate) sub_type_name: Option<String>,
}

impl From<QuotaSnapshotRow> for QuotaSnapshotResponse {
    fn from(value: QuotaSnapshotRow) -> Self {
        Self {
            captured_at: value.captured_at,
            amount_limit: value.amount_limit,
            used_amount: value.used_amount,
            remaining_amount: value.remaining_amount,
            period: value.period,
            period_reset_time: value.period_reset_time,
            expire_time: value.expire_time,
            is_active: value.is_active.unwrap_or(0) != 0,
            total_cost: value.total_cost,
            total_requests: value.total_requests,
            total_tokens: value.total_tokens,
            last_request_time: value.last_request_time,
            billing_type: value.billing_type,
            remaining_count: value.remaining_count,
            used_count: value.used_count,
            sub_type_name: value.sub_type_name,
        }
    }
}

impl QuotaSnapshotResponse {
    pub(crate) async fn fetch_latest(pool: &Pool<Sqlite>) -> Result<Option<Self>> {
        let row = sqlx::query_as::<_, QuotaSnapshotRow>(
            r#"
            SELECT
                captured_at,
                amount_limit,
                used_amount,
                remaining_amount,
                period,
                period_reset_time,
                expire_time,
                is_active,
                total_cost,
                total_requests,
                total_tokens,
                last_request_time,
                billing_type,
                remaining_count,
                used_count,
                sub_type_name
            FROM codex_quota_snapshots
            ORDER BY captured_at DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(pool)
        .await?;

        Ok(row.map(Into::into))
    }

    pub(crate) fn degraded_default() -> Self {
        Self {
            captured_at: format_utc_iso(Utc::now()),
            amount_limit: None,
            used_amount: None,
            remaining_amount: None,
            period: None,
            period_reset_time: None,
            expire_time: None,
            is_active: false,
            total_cost: 0.0,
            total_requests: 0,
            total_tokens: 0,
            last_request_time: None,
            billing_type: None,
            remaining_count: None,
            used_count: None,
            sub_type_name: None,
        }
    }
}
