use super::*;
use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use tokio::sync::watch;

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
    pub(crate) upstream_account_id: Option<i64>,
    #[sqlx(default)]
    pub(crate) upstream_account_name: Option<String>,
    #[sqlx(default)]
    pub(crate) model: Option<String>,
    #[sqlx(default)]
    pub(crate) request_model: Option<String>,
    #[sqlx(default)]
    pub(crate) response_model: Option<String>,
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
    pub(crate) connect_latency_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) first_byte_latency_ms: Option<f64>,
    #[sqlx(default)]
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

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageCostBreakdownResponse {
    pub(crate) input: f64,
    pub(crate) cache_write: f64,
    pub(crate) cache_read: f64,
    pub(crate) output: f64,
    pub(crate) reasoning: f64,
    pub(crate) unknown: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageBreakdownResponse {
    pub(crate) cache_write_tokens: i64,
    pub(crate) cache_read_tokens: i64,
    pub(crate) output_tokens: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) costs: Option<UsageCostBreakdownResponse>,
    pub(crate) models: Vec<UsageBreakdownModelResponse>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Serialize)]
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
    pub(crate) cache_input_tokens: i64,
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
}

#[derive(Debug, FromRow)]
pub(crate) struct UpstreamAccountUsageHourlyRollupRecord {
    pub(crate) bucket_start_epoch: i64,
    pub(crate) request_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) cache_input_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) non_success_cost: f64,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationsResponse {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) snapshot_at: Option<String>,
    pub(crate) selection_mode: PromptCacheConversationSelectionMode,
    pub(crate) selected_limit: Option<i64>,
    pub(crate) selected_activity_hours: Option<i64>,
    pub(crate) selected_activity_minutes: Option<i64>,
    pub(crate) implicit_filter: PromptCacheConversationImplicitFilter,
    pub(crate) total_matched: Option<i64>,
    pub(crate) has_more: bool,
    pub(crate) next_cursor: Option<String>,
    pub(crate) conversations: Vec<PromptCacheConversationResponse>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PromptCacheConversationSelectionMode {
    Count,
    ActivityWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PromptCacheConversationSelection {
    Count(i64),
    ActivityWindowHours(i64),
    ActivityWindowMinutes(i64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PromptCacheConversationDetailLevel {
    Full,
    Compact,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PromptCacheConversationsRequest {
    pub(crate) selection: PromptCacheConversationSelection,
    pub(crate) detail_level: PromptCacheConversationDetailLevel,
    pub(crate) recent_invocation_limit: Option<i64>,
    pub(crate) page_size: Option<i64>,
    pub(crate) cursor: Option<String>,
    pub(crate) snapshot_at: Option<String>,
}

impl PromptCacheConversationsRequest {
    pub(crate) fn legacy(selection: PromptCacheConversationSelection) -> Self {
        Self {
            selection,
            detail_level: PromptCacheConversationDetailLevel::Full,
            recent_invocation_limit: None,
            page_size: None,
            cursor: None,
            snapshot_at: None,
        }
    }

    pub(crate) fn uses_legacy_cache(&self) -> bool {
        self.detail_level == PromptCacheConversationDetailLevel::Full
            && self.recent_invocation_limit.is_none()
            && self.page_size.is_none()
            && self.cursor.is_none()
            && self.snapshot_at.is_none()
    }
}

impl PromptCacheConversationSelection {
    pub(crate) fn selection_mode(self) -> PromptCacheConversationSelectionMode {
        match self {
            Self::Count(_) => PromptCacheConversationSelectionMode::Count,
            Self::ActivityWindowHours(_) | Self::ActivityWindowMinutes(_) => {
                PromptCacheConversationSelectionMode::ActivityWindow
            }
        }
    }

    pub(crate) fn activity_window_duration(self) -> ChronoDuration {
        match self {
            Self::Count(_) => ChronoDuration::hours(24),
            Self::ActivityWindowHours(hours) => ChronoDuration::hours(hours),
            Self::ActivityWindowMinutes(minutes) => ChronoDuration::minutes(minutes),
        }
    }

    pub(crate) fn display_limit(self) -> i64 {
        match self {
            Self::Count(limit) => limit,
            Self::ActivityWindowHours(_) | Self::ActivityWindowMinutes(_) => {
                PROMPT_CACHE_CONVERSATION_ACTIVITY_MODE_LIMIT
            }
        }
    }

    pub(crate) fn selected_limit(self) -> Option<i64> {
        match self {
            Self::Count(limit) => Some(limit),
            Self::ActivityWindowHours(_) | Self::ActivityWindowMinutes(_) => None,
        }
    }

    pub(crate) fn selected_activity_hours(self) -> Option<i64> {
        match self {
            Self::Count(_) => None,
            Self::ActivityWindowHours(hours) => Some(hours),
            Self::ActivityWindowMinutes(_) => None,
        }
    }

    pub(crate) fn selected_activity_minutes(self) -> Option<i64> {
        match self {
            Self::Count(_) | Self::ActivityWindowHours(_) => None,
            Self::ActivityWindowMinutes(minutes) => Some(minutes),
        }
    }

    pub(crate) fn implicit_filter(
        self,
        filtered_count: i64,
    ) -> PromptCacheConversationImplicitFilter {
        let kind = if filtered_count > 0 {
            Some(match self {
                Self::Count(_) => PromptCacheConversationImplicitFilterKind::InactiveOutside24h,
                Self::ActivityWindowHours(_) | Self::ActivityWindowMinutes(_) => {
                    PromptCacheConversationImplicitFilterKind::CappedTo50
                }
            })
        } else {
            None
        };

        PromptCacheConversationImplicitFilter {
            kind,
            filtered_count: filtered_count.max(0),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationImplicitFilter {
    pub(crate) kind: Option<PromptCacheConversationImplicitFilterKind>,
    pub(crate) filtered_count: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PromptCacheConversationImplicitFilterKind {
    InactiveOutside24h,
    CappedTo50,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationResponse {
    pub(crate) prompt_cache_key: String,
    pub(crate) request_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) created_at: String,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) last_activity_at: String,
    #[serde(
        serialize_with = "serialize_opt_local_or_utc_to_utc_iso",
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) last_terminal_at: Option<String>,
    #[serde(
        serialize_with = "serialize_opt_local_or_utc_to_utc_iso",
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) last_in_flight_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cursor: Option<String>,
    pub(crate) has_encrypted_session_owner: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) encrypted_owner_account_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) encrypted_owner_account_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) encrypted_owner_group_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) manual_binding: Option<PromptCacheConversationManualBindingResponse>,
    pub(crate) upstream_accounts: Vec<PromptCacheConversationUpstreamAccountResponse>,
    pub(crate) recent_invocations: Vec<PromptCacheConversationInvocationPreviewResponse>,
    pub(crate) last24h_requests: Vec<PromptCacheConversationRequestPointResponse>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationManualBindingResponse {
    pub(crate) binding_kind: String,
    pub(crate) group_name: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationInvocationPreviewResponse {
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    pub(crate) prompt_cache_key: Option<String>,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) occurred_at: String,
    pub(crate) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) live_phase: Option<String>,
    pub(crate) failure_class: Option<String>,
    pub(crate) route_mode: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) request_model: Option<String>,
    pub(crate) response_model: Option<String>,
    pub(crate) total_tokens: i64,
    pub(crate) cost: Option<f64>,
    pub(crate) proxy_display_name: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) upstream_account_plan_type: Option<String>,
    pub(crate) endpoint: Option<String>,
    pub(crate) compaction_request_kind: Option<String>,
    pub(crate) compaction_response_kind: Option<String>,
    pub(crate) image_intent: Option<String>,
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
    pub(crate) is_actionable: Option<bool>,
    pub(crate) response_content_encoding: Option<String>,
    pub(crate) transport: Option<String>,
    pub(crate) requested_service_tier: Option<String>,
    pub(crate) service_tier: Option<String>,
    pub(crate) billing_service_tier: Option<String>,
    pub(crate) t_req_read_ms: Option<f64>,
    pub(crate) t_req_parse_ms: Option<f64>,
    pub(crate) t_upstream_connect_ms: Option<f64>,
    pub(crate) t_upstream_ttfb_ms: Option<f64>,
    pub(crate) t_upstream_stream_ms: Option<f64>,
    pub(crate) t_resp_parse_ms: Option<f64>,
    pub(crate) t_persist_ms: Option<f64>,
    pub(crate) t_total_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationUpstreamAccountResponse {
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) request_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) last_activity_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationRequestPointResponse {
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) occurred_at: String,
    pub(crate) status: String,
    pub(crate) is_success: bool,
    pub(crate) outcome: String,
    pub(crate) request_tokens: i64,
    pub(crate) cumulative_tokens: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct PromptCacheConversationsCacheEntry {
    pub(crate) cached_at: Instant,
    pub(crate) generation: u64,
    pub(crate) response: PromptCacheConversationsResponse,
}

#[derive(Debug)]
pub(crate) struct PromptCacheConversationInFlight {
    pub(crate) signal: watch::Sender<bool>,
    pub(crate) generation: u64,
}

#[derive(Debug, Default)]
pub(crate) struct PromptCacheConversationsCacheState {
    pub(crate) entries:
        HashMap<PromptCacheConversationSelection, PromptCacheConversationsCacheEntry>,
    pub(crate) in_flight:
        HashMap<PromptCacheConversationSelection, PromptCacheConversationInFlight>,
    pub(crate) generation: u64,
}

#[derive(Debug)]
pub(crate) struct PromptCacheConversationFlightGuard {
    pub(crate) cache: Arc<Mutex<PromptCacheConversationsCacheState>>,
    pub(crate) selection: PromptCacheConversationSelection,
    pub(crate) generation: u64,
    pub(crate) active: bool,
}

impl PromptCacheConversationFlightGuard {
    pub(crate) fn new(
        cache: Arc<Mutex<PromptCacheConversationsCacheState>>,
        selection: PromptCacheConversationSelection,
        generation: u64,
    ) -> Self {
        Self {
            cache,
            selection,
            generation,
            active: true,
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for PromptCacheConversationFlightGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }

        let cache = self.cache.clone();
        let selection = self.selection;
        let generation = self.generation;
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let mut state = cache.lock().await;
                if let Some(in_flight) = state.in_flight.remove(&selection) {
                    if in_flight.generation != generation {
                        state.in_flight.insert(selection, in_flight);
                        return;
                    }
                    let _ = in_flight.signal.send(true);
                }
            });
            return;
        }

        if let Ok(mut state) = cache.try_lock()
            && let Some(in_flight) = state.in_flight.remove(&selection)
        {
            if in_flight.generation != generation {
                state.in_flight.insert(selection, in_flight);
                return;
            }
            let _ = in_flight.signal.send(true);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct DashboardActivitySnapshotSelection {
    pub(crate) range: String,
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) time_zone: String,
    pub(crate) source_scope: String,
    pub(crate) invocation_snapshot_id: i64,
    pub(crate) live_activity_key: String,
    pub(crate) recent_limit: usize,
    pub(crate) include_accounts: bool,
    pub(crate) include_recent: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct DashboardActivitySnapshotCacheEntry {
    pub(crate) cached_at: Instant,
    pub(crate) response: DashboardActivitySnapshot,
}

#[derive(Debug)]
pub(crate) struct DashboardActivitySnapshotInFlight {
    pub(crate) signal: watch::Sender<bool>,
    pub(crate) waiter_count: usize,
}

#[derive(Debug, Default)]
pub(crate) struct DashboardActivitySnapshotCacheState {
    pub(crate) entries:
        HashMap<DashboardActivitySnapshotSelection, DashboardActivitySnapshotCacheEntry>,
    pub(crate) in_flight:
        HashMap<DashboardActivitySnapshotSelection, DashboardActivitySnapshotInFlight>,
}

#[derive(Debug)]
pub(crate) struct DashboardActivitySnapshotFlightGuard {
    pub(crate) cache: Arc<Mutex<DashboardActivitySnapshotCacheState>>,
    pub(crate) selection: DashboardActivitySnapshotSelection,
    pub(crate) active: bool,
}

impl DashboardActivitySnapshotFlightGuard {
    pub(crate) fn new(
        cache: Arc<Mutex<DashboardActivitySnapshotCacheState>>,
        selection: DashboardActivitySnapshotSelection,
    ) -> Self {
        Self {
            cache,
            selection,
            active: true,
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for DashboardActivitySnapshotFlightGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }

        let cache = self.cache.clone();
        let selection = self.selection.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let mut state = cache.lock().await;
                if let Some(in_flight) = state.in_flight.remove(&selection) {
                    let _ = in_flight.signal.send(true);
                }
            });
            return;
        }

        if let Ok(mut state) = cache.try_lock()
            && let Some(in_flight) = state.in_flight.remove(&selection)
        {
            let _ = in_flight.signal.send(true);
        }
    }
}

pub(crate) async fn invalidate_prompt_cache_conversations_cache(
    cache: &Arc<Mutex<PromptCacheConversationsCacheState>>,
) {
    let in_flight = {
        let mut state = cache.lock().await;
        state.generation = state.generation.wrapping_add(1);
        state.entries.clear();
        std::mem::take(&mut state.in_flight)
    };

    for flight in in_flight.into_values() {
        let _ = flight.signal.send(true);
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ParsedUsage {
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    pub(crate) total_tokens: Option<i64>,
}

#[derive(Debug, Clone, Copy, Default)]
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

#[derive(Debug, Clone, Default)]
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

#[derive(Debug, Clone, Copy)]
pub(crate) struct StageTimings {
    pub(crate) t_total_ms: f64,
    pub(crate) t_req_read_ms: f64,
    pub(crate) t_req_parse_ms: f64,
    pub(crate) t_upstream_connect_ms: f64,
    pub(crate) t_upstream_ttfb_ms: f64,
    pub(crate) t_upstream_stream_ms: f64,
    pub(crate) t_resp_parse_ms: f64,
    pub(crate) t_persist_ms: f64,
}

#[derive(Debug, Clone)]
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
    ImageGenerations,
    ImageEdits,
}

impl ProxyCaptureTarget {
    pub(crate) fn endpoint(self) -> &'static str {
        match self {
            Self::ChatCompletions => "/v1/chat/completions",
            Self::Responses => "/v1/responses",
            Self::ResponsesCompact => "/v1/responses/compact",
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
    pub(crate) is_actionable: Option<i64>,
    pub(crate) proxy_display_name: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) upstream_account_plan_type: Option<String>,
    pub(crate) response_content_encoding: Option<String>,
    pub(crate) transport: Option<String>,
    pub(crate) requested_service_tier: Option<String>,
    pub(crate) service_tier: Option<String>,
    pub(crate) billing_service_tier: Option<String>,
    pub(crate) t_req_read_ms: Option<f64>,
    pub(crate) t_req_parse_ms: Option<f64>,
    pub(crate) t_upstream_connect_ms: Option<f64>,
    pub(crate) t_upstream_ttfb_ms: Option<f64>,
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
    pub(crate) transport: Option<String>,
    pub(crate) requested_service_tier: Option<String>,
    pub(crate) service_tier: Option<String>,
    pub(crate) billing_service_tier: Option<String>,
    pub(crate) t_req_read_ms: Option<f64>,
    pub(crate) t_req_parse_ms: Option<f64>,
    pub(crate) t_upstream_connect_ms: Option<f64>,
    pub(crate) t_upstream_ttfb_ms: Option<f64>,
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
    pub(crate) status: Option<String>,
    // Kept for compatibility so stale /records URLs with `?proxy=...` deserialize cleanly,
    // but records queries intentionally ignore this field.
    #[allow(dead_code)]
    pub(crate) proxy: Option<String>,
    pub(crate) endpoint: Option<String>,
    pub(crate) request_id: Option<String>,
    pub(crate) failure_class: Option<String>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) prompt_cache_key: Option<String>,
    pub(crate) sticky_key: Option<String>,
    pub(crate) upstream_scope: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
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

#[derive(Debug, Deserialize)]
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
    pub(crate) current_avg_total_ms: Option<f64>,
    pub(crate) model_performance: ModelPerformanceResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardActivityResponse {
    pub(crate) range: String,
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) snapshot_id: i64,
    pub(crate) live_revision: u64,
    pub(crate) rate_window: DashboardActivityRateWindowResponse,
    pub(crate) summary: DashboardActivitySummaryResponse,
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
    pub(crate) avg_total_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current_first_response_byte_total_avg_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current_avg_total_ms: Option<f64>,
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
    pub(crate) avg_total_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current_first_response_byte_total_avg_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current_avg_total_ms: Option<f64>,
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PerfStageStats {
    pub(crate) stage: String,
    pub(crate) count: i64,
    pub(crate) avg_ms: f64,
    pub(crate) p50_ms: f64,
    pub(crate) p90_ms: f64,
    pub(crate) p99_ms: f64,
    pub(crate) max_ms: f64,
}
