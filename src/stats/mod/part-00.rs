use super::*;

pub(crate) const STATS_SUCCESS_LIKE_SQL: &str = "(LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'completed', 'warning_success') OR (LOWER(TRIM(COALESCE(status, ''))) = 'http_200' AND TRIM(COALESCE(error_message, '')) = ''))";
pub(crate) const STATS_TERMINAL_STATUS_SQL: &str =
    "(LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending'))";
pub(crate) const INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET: &str =
    "codex_invocations_summary_rollup_v2_live_cursor";
pub(crate) const MISSING_INVOCATION_ARCHIVE_REPAIR_PREFIX: &str =
    "completed invocation archive is missing during summary rollup repair";

#[derive(Debug, Clone)]
pub(crate) enum SummaryWindow {
    All,
    Current(i64),
    Duration(ChronoDuration),
    Calendar(String),
    PreviousFullDays(i64),
}

pub(crate) fn stats_success_failure_select_sql() -> String {
    format!(
        "COUNT(*) AS total_count, \
         COALESCE(SUM(CASE WHEN {success_like} AND {resolved_failure} = 'none' THEN 1 ELSE 0 END), 0) AS success_count, \
         COALESCE(SUM(CASE WHEN {terminal_status} AND {resolved_failure} IN ('service_failure', 'client_failure', 'client_abort') THEN 1 ELSE 0 END), 0) AS failure_count, \
         COALESCE(SUM(cost), 0.0) AS total_cost, \
         COALESCE(SUM(total_tokens), 0) AS total_tokens, \
         COALESCE(SUM(CASE WHEN {terminal_status} AND {resolved_failure} IN ('service_failure', 'client_failure', 'client_abort') THEN COALESCE(cost, 0.0) ELSE 0.0 END), 0.0) AS non_success_cost",
        success_like = STATS_SUCCESS_LIKE_SQL,
        terminal_status = STATS_TERMINAL_STATUS_SQL,
        resolved_failure = crate::api::INVOCATION_RESOLVED_FAILURE_CLASS_SQL,
    )
}

pub(crate) fn is_missing_invocation_summary_archive_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .to_string()
            .contains(MISSING_INVOCATION_ARCHIVE_REPAIR_PREFIX)
    })
}

pub(crate) fn is_unreadable_invocation_summary_archive_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        let message = cause.to_string();
        message.contains("failed to decompress archive batch ")
            || message.contains("failed to open archive batch ")
            || message.contains("database disk image is malformed")
    })
}

#[derive(Debug, Clone)]
pub(crate) enum StatsFilter {
    All,
    Since(DateTime<Utc>),
    Range(DateTime<Utc>, DateTime<Utc>),
    RecentLimit(i64),
}

#[derive(Debug, FromRow)]
pub(crate) struct TimeseriesRecord {
    pub(crate) occurred_at: String,
    pub(crate) status: Option<String>,
    pub(crate) total_tokens: Option<i64>,
    pub(crate) cost: Option<f64>,
    pub(crate) t_req_read_ms: Option<f64>,
    pub(crate) t_req_parse_ms: Option<f64>,
    pub(crate) t_upstream_connect_ms: Option<f64>,
    pub(crate) t_upstream_ttfb_ms: Option<f64>,
    pub(crate) first_token_ms: Option<f64>,
}

#[derive(Debug, FromRow)]
pub(crate) struct StatsDeltaRecord {
    pub(crate) captured_at_epoch: i64,
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
}

#[derive(Debug, FromRow)]
pub(crate) struct InvocationHourlyRollupRecord {
    pub(crate) bucket_start_epoch: i64,
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
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

#[derive(Debug, FromRow)]
pub(crate) struct InvocationFailureHourlyRollupRecord {
    pub(crate) failure_class: String,
    pub(crate) is_actionable: i64,
    pub(crate) error_category: String,
    pub(crate) failure_count: i64,
}

#[derive(Debug, FromRow)]
pub(crate) struct ProxyPerfStageHourlyRollupRecord {
    pub(crate) bucket_start_epoch: i64,
    pub(crate) stage: String,
    pub(crate) sample_count: i64,
    pub(crate) sum_ms: f64,
    pub(crate) max_ms: f64,
    pub(crate) histogram: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct KeyedConversationHourlyRollupRecord {
    pub(crate) request_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) first_seen_at: String,
    pub(crate) last_seen_at: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct ForwardProxyAttemptHourlyRollupRecord {
    pub(crate) bucket_start_epoch: i64,
    pub(crate) attempts: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) latency_sample_count: i64,
    pub(crate) latency_sum_ms: f64,
    pub(crate) latency_max_ms: f64,
}

pub(crate) const APPROX_HISTOGRAM_BUCKETS_MS: [f64; 20] = [
    5.0, 10.0, 20.0, 50.0, 100.0, 150.0, 200.0, 300.0, 500.0, 750.0, 1_000.0, 1_500.0, 2_000.0,
    3_000.0, 5_000.0, 10_000.0, 20_000.0, 30_000.0, 60_000.0, 180_000.0,
];

pub(crate) type ApproxHistogramCounts = Vec<i64>;

pub(crate) fn empty_approx_histogram() -> ApproxHistogramCounts {
    vec![0; APPROX_HISTOGRAM_BUCKETS_MS.len() + 1]
}

pub(crate) fn decode_approx_histogram(raw: &str) -> ApproxHistogramCounts {
    let mut counts = serde_json::from_str::<Vec<i64>>(raw).unwrap_or_default();
    let expected_len = APPROX_HISTOGRAM_BUCKETS_MS.len() + 1;
    if counts.len() < expected_len {
        counts.resize(expected_len, 0);
    } else if counts.len() > expected_len {
        counts.truncate(expected_len);
    }
    counts
}

pub(crate) fn encode_approx_histogram(counts: &[i64]) -> Result<String> {
    Ok(serde_json::to_string(counts)?)
}

pub(crate) fn add_approx_histogram_sample(counts: &mut ApproxHistogramCounts, value_ms: f64) {
    if !value_ms.is_finite() || value_ms < 0.0 {
        return;
    }
    let index = APPROX_HISTOGRAM_BUCKETS_MS
        .iter()
        .position(|upper| value_ms <= *upper)
        .unwrap_or(APPROX_HISTOGRAM_BUCKETS_MS.len());
    if let Some(slot) = counts.get_mut(index) {
        *slot += 1;
    }
}

pub(crate) fn subtract_approx_histogram_sample(counts: &mut ApproxHistogramCounts, value_ms: f64) {
    if !value_ms.is_finite() || value_ms < 0.0 {
        return;
    }
    let index = APPROX_HISTOGRAM_BUCKETS_MS
        .iter()
        .position(|upper| value_ms <= *upper)
        .unwrap_or(APPROX_HISTOGRAM_BUCKETS_MS.len());
    if let Some(slot) = counts.get_mut(index)
        && *slot > 0
    {
        *slot -= 1;
    }
}

pub(crate) fn normalize_non_negative_timing_value(value: Option<f64>) -> Option<f64> {
    let value = value?;
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    Some(value)
}

pub(crate) fn validated_proxy_perf_stage_rollup(
    stage: &str,
    sample_count: i64,
    sum_ms: f64,
    max_ms: f64,
    mut histogram: ApproxHistogramCounts,
) -> Option<(i64, f64, f64, ApproxHistogramCounts)> {
    if sample_count <= 0
        || !is_valid_perf_stage_sample(stage, sum_ms)
        || !is_valid_perf_stage_sample(stage, max_ms)
    {
        return None;
    }
    if histogram.is_empty() {
        histogram = empty_approx_histogram();
    }
    for count in &mut histogram {
        *count = (*count).max(0);
    }
    Some((sample_count, sum_ms, max_ms, histogram))
}

#[cfg(test)]
mod perf_stage_rollup_validation_tests {
    use super::*;

    #[test]
    fn persisted_rollups_reject_invalid_values_and_normalize_histogram_counts() {
        assert!(
            validated_proxy_perf_stage_rollup(
                "upstreamStream",
                1,
                0.0,
                0.0,
                empty_approx_histogram(),
            )
            .is_none()
        );
        assert!(
            validated_proxy_perf_stage_rollup(
                "total",
                1,
                f64::INFINITY,
                1.0,
                empty_approx_histogram(),
            )
            .is_none()
        );

        let mut histogram = empty_approx_histogram();
        histogram[0] = -2;
        histogram[1] = 3;
        let (_, _, _, normalized) =
            validated_proxy_perf_stage_rollup("total", 1, 3.0, 3.0, histogram)
                .expect("valid rollup");
        assert_eq!(normalized[0], 0);
        assert_eq!(normalized[1], 3);
    }
}

pub(crate) fn resolve_first_response_byte_total_ms(
    t_req_read_ms: Option<f64>,
    t_req_parse_ms: Option<f64>,
    t_upstream_connect_ms: Option<f64>,
    t_upstream_ttfb_ms: Option<f64>,
) -> Option<f64> {
    let t_upstream_ttfb_ms = normalize_non_negative_timing_value(t_upstream_ttfb_ms)?;
    if t_upstream_ttfb_ms <= 0.0 {
        return None;
    }
    Some(
        normalize_non_negative_timing_value(t_req_read_ms)?
            + normalize_non_negative_timing_value(t_req_parse_ms)?
            + normalize_non_negative_timing_value(t_upstream_connect_ms)?
            + t_upstream_ttfb_ms,
    )
}

pub(crate) fn merge_approx_histogram_into(
    target: &mut ApproxHistogramCounts,
    source: &[i64],
) -> Result<()> {
    if target.len() != source.len() {
        bail!(
            "histogram length mismatch: target={}, source={}",
            target.len(),
            source.len()
        );
    }
    for (target_count, source_count) in target.iter_mut().zip(source.iter()) {
        *target_count += *source_count;
    }
    Ok(())
}

pub(crate) fn approx_histogram_percentile_ms(counts: &[i64], percentile: f64) -> Option<f64> {
    if counts.is_empty() {
        return None;
    }
    let total: i64 = counts.iter().copied().sum();
    if total <= 0 {
        return None;
    }
    let mut threshold = ((total as f64) * percentile).ceil() as i64;
    if threshold <= 0 {
        threshold = 1;
    }
    let mut seen = 0_i64;
    for (index, count) in counts.iter().copied().enumerate() {
        seen += count;
        if seen < threshold {
            continue;
        }
        if index < APPROX_HISTOGRAM_BUCKETS_MS.len() {
            return Some(APPROX_HISTOGRAM_BUCKETS_MS[index]);
        }
        return APPROX_HISTOGRAM_BUCKETS_MS.last().copied();
    }
    APPROX_HISTOGRAM_BUCKETS_MS.last().copied()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct BucketAggregate {
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) in_flight_count: i64,
    pub(crate) in_flight_phase_counts: InvocationPhaseCountsResponse,
    pub(crate) total_tokens: i64,
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) cache_input_tokens: i64,
    pub(crate) reasoning_tokens: i64,
    #[serde(default)]
    pub(crate) token_components_observed: bool,
    #[serde(default)]
    pub(crate) token_component_incomplete_count: i64,
    pub(crate) total_cost: f64,
    pub(crate) non_success_cost: f64,
    pub(crate) total_latency_sum_ms: f64,
    pub(crate) total_latency_sample_count: i64,
    #[serde(default, skip_serializing)]
    pub(crate) total_latency_values: Vec<f64>,
    pub(crate) first_byte_ttfb_sum_ms: f64,
    pub(crate) first_byte_ttfb_values: Vec<f64>,
    pub(crate) first_byte_histogram: ApproxHistogramCounts,
    pub(crate) first_byte_sample_count: i64,
    pub(crate) first_response_byte_total_sum_ms: f64,
    pub(crate) first_response_byte_total_values: Vec<f64>,
    pub(crate) first_response_byte_total_histogram: ApproxHistogramCounts,
    pub(crate) first_response_byte_total_sample_count: i64,
    pub(crate) first_token_sum_ms: f64,
    pub(crate) first_token_values: Vec<f64>,
    pub(crate) first_token_histogram: ApproxHistogramCounts,
    pub(crate) first_token_sample_count: i64,
}

impl BucketAggregate {
    fn validated_total_latency_value(value: Option<f64>) -> Option<f64> {
        let value = value?;
        if !value.is_finite() || value < 0.0 {
            return None;
        }
        Some(value)
    }

    pub(crate) fn record_total_latency_sample(&mut self, total_ms: Option<f64>) {
        let Some(value) = Self::validated_total_latency_value(total_ms) else {
            return;
        };
        self.total_latency_sample_count += 1;
        self.total_latency_sum_ms += value;
        self.total_latency_values.push(value);
    }

    pub(crate) fn total_latency_avg_ms(&self) -> Option<f64> {
        if self.total_latency_sample_count <= 0 {
            return None;
        }
        Some(self.total_latency_sum_ms / self.total_latency_sample_count as f64)
    }

    fn validated_success_ttfb_value(status: Option<&str>, ttfb_ms: Option<f64>) -> Option<f64> {
        if !crate::maintenance::invocation_status_is_success_like(status, None) {
            return None;
        }
        let value = ttfb_ms?;
        if !value.is_finite() || value <= 0.0 {
            return None;
        }
        Some(value)
    }

    fn record_first_byte_ttfb_value(&mut self, value: f64) {
        self.first_byte_sample_count += 1;
        self.first_byte_ttfb_sum_ms += value;
        self.first_byte_ttfb_values.push(value);
        if self.first_byte_histogram.is_empty() {
            self.first_byte_histogram = empty_approx_histogram();
        }
        add_approx_histogram_sample(&mut self.first_byte_histogram, value);
    }

    fn record_first_response_byte_total_value(&mut self, value: f64) {
        self.first_response_byte_total_sample_count += 1;
        self.first_response_byte_total_sum_ms += value;
        self.first_response_byte_total_values.push(value);
        if self.first_response_byte_total_histogram.is_empty() {
            self.first_response_byte_total_histogram = empty_approx_histogram();
        }
        add_approx_histogram_sample(&mut self.first_response_byte_total_histogram, value);
    }

    fn record_first_token_value(&mut self, value: f64) {
        self.first_token_sample_count += 1;
        self.first_token_sum_ms += value;
        self.first_token_values.push(value);
        if self.first_token_histogram.is_empty() {
            self.first_token_histogram = empty_approx_histogram();
        }
        add_approx_histogram_sample(&mut self.first_token_histogram, value);
    }

    pub(crate) fn record_first_token_sample(&mut self, first_token_ms: Option<f64>) {
        let Some(value) = first_token_ms.filter(|value| value.is_finite() && *value >= 0.0) else {
            return;
        };
        self.record_first_token_value(value);
    }

    pub(crate) fn remove_exact_first_token_sample(&mut self, first_token_ms: Option<f64>) {
        let Some(value) = first_token_ms.filter(|value| value.is_finite() && *value >= 0.0) else {
            return;
        };
        self.first_token_sample_count = self.first_token_sample_count.saturating_sub(1);
        self.first_token_sum_ms = (self.first_token_sum_ms - value).max(0.0);
        if let Some(index) = self
            .first_token_values
            .iter()
            .position(|sample| (*sample - value).abs() <= f64::EPSILON)
        {
            self.first_token_values.swap_remove(index);
        }
        subtract_approx_histogram_sample(&mut self.first_token_histogram, value);
    }

    pub(crate) fn record_ttfb_sample(&mut self, status: Option<&str>, ttfb_ms: Option<f64>) {
        let Some(value) = Self::validated_success_ttfb_value(status, ttfb_ms) else {
            return;
        };
        self.record_first_byte_ttfb_value(value);
    }

    pub(crate) fn record_exact_ttfb_sample(&mut self, status: Option<&str>, ttfb_ms: Option<f64>) {
        let Some(value) = Self::validated_success_ttfb_value(status, ttfb_ms) else {
            return;
        };
        self.record_first_byte_ttfb_value(value);
    }

    pub(crate) fn record_first_response_byte_total_sample(
        &mut self,
        t_req_read_ms: Option<f64>,
        t_req_parse_ms: Option<f64>,
        t_upstream_connect_ms: Option<f64>,
        t_upstream_ttfb_ms: Option<f64>,
    ) {
        let Some(value) = resolve_first_response_byte_total_ms(
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
        ) else {
            return;
        };
        self.record_first_response_byte_total_value(value);
    }

    pub(crate) fn record_exact_first_response_byte_total_sample(
        &mut self,
        t_req_read_ms: Option<f64>,
        t_req_parse_ms: Option<f64>,
        t_upstream_connect_ms: Option<f64>,
        t_upstream_ttfb_ms: Option<f64>,
    ) {
        let Some(value) = resolve_first_response_byte_total_ms(
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
        ) else {
            return;
        };
        self.record_first_response_byte_total_value(value);
    }

    pub(crate) fn remove_exact_first_response_byte_total_sample(
        &mut self,
        t_req_read_ms: Option<f64>,
        t_req_parse_ms: Option<f64>,
        t_upstream_connect_ms: Option<f64>,
        t_upstream_ttfb_ms: Option<f64>,
    ) {
        let Some(value) = resolve_first_response_byte_total_ms(
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
        ) else {
            return;
        };
        self.first_response_byte_total_sample_count = self
            .first_response_byte_total_sample_count
            .saturating_sub(1);
        self.first_response_byte_total_sum_ms =
            (self.first_response_byte_total_sum_ms - value).max(0.0);
        if let Some(index) = self
            .first_response_byte_total_values
            .iter()
            .position(|sample| (*sample - value).abs() <= f64::EPSILON)
        {
            self.first_response_byte_total_values.swap_remove(index);
        }
        subtract_approx_histogram_sample(&mut self.first_response_byte_total_histogram, value);
    }

    pub(crate) fn first_byte_avg_ms(&self) -> Option<f64> {
        if self.first_byte_sample_count <= 0 {
            return None;
        }
        Some(self.first_byte_ttfb_sum_ms / self.first_byte_sample_count as f64)
    }

    pub(crate) fn first_byte_p95_ms(&self) -> Option<f64> {
        if self.first_byte_ttfb_values.is_empty() {
            return approx_histogram_percentile_ms(&self.first_byte_histogram, 0.95);
        }
        let histogram_total: i64 = self.first_byte_histogram.iter().copied().sum();
        if histogram_total > self.first_byte_ttfb_values.len() as i64 {
            return approx_histogram_percentile_ms(&self.first_byte_histogram, 0.95);
        }
        let mut sorted = self.first_byte_ttfb_values.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Some(percentile_sorted_f64(&sorted, 0.95))
    }

    pub(crate) fn first_response_byte_total_avg_ms(&self) -> Option<f64> {
        if self.first_response_byte_total_sample_count <= 0 {
            return None;
        }
        Some(
            self.first_response_byte_total_sum_ms
                / self.first_response_byte_total_sample_count as f64,
        )
    }

    pub(crate) fn first_response_byte_total_p95_ms(&self) -> Option<f64> {
        if self.first_response_byte_total_values.is_empty() {
            return approx_histogram_percentile_ms(&self.first_response_byte_total_histogram, 0.95);
        }
        let histogram_total: i64 = self
            .first_response_byte_total_histogram
            .iter()
            .copied()
            .sum();
        if histogram_total > self.first_response_byte_total_values.len() as i64 {
            return approx_histogram_percentile_ms(&self.first_response_byte_total_histogram, 0.95);
        }
        let mut sorted = self.first_response_byte_total_values.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Some(percentile_sorted_f64(&sorted, 0.95))
    }

    pub(crate) fn first_token_avg_ms(&self) -> Option<f64> {
        (self.first_token_sample_count > 0)
            .then_some(self.first_token_sum_ms / self.first_token_sample_count as f64)
    }

    pub(crate) fn first_token_p95_ms(&self) -> Option<f64> {
        if self.first_token_values.is_empty() {
            return approx_histogram_percentile_ms(&self.first_token_histogram, 0.95);
        }
        let histogram_total: i64 = self.first_token_histogram.iter().copied().sum();
        if histogram_total > self.first_token_values.len() as i64 {
            return approx_histogram_percentile_ms(&self.first_token_histogram, 0.95);
        }
        let mut sorted = self.first_token_values.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Some(percentile_sorted_f64(&sorted, 0.95))
    }
}

pub(crate) fn default_range() -> String {
    "1d".to_string()
}

pub(crate) fn format_naive(dt: NaiveDateTime) -> String {
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

pub(crate) fn format_naive_precise(dt: NaiveDateTime) -> String {
    dt.format("%Y-%m-%d %H:%M:%S%.f").to_string()
}

pub(crate) fn parse_reporting_tz(time_zone: Option<&str>) -> Result<Tz> {
    let tz_name = time_zone.unwrap_or("Asia/Shanghai");
    tz_name
        .parse::<Tz>()
        .with_context(|| format!("invalid timeZone: {tz_name}"))
}

// `codex_invocations.occurred_at` is stored as a naive Asia/Shanghai timestamp string
// (e.g. "2026-01-21 01:02:15"). For lexicographic filtering to work correctly,
// we must bind the lower bound using the same representation.
pub(crate) fn db_occurred_at_lower_bound(start_utc: DateTime<Utc>) -> String {
    let shanghai = start_utc.with_timezone(&Shanghai);
    format_naive(shanghai.naive_local())
}

pub(crate) fn exclusive_epoch_upper_bound(end_utc: DateTime<Utc>) -> i64 {
    end_utc.timestamp()
        + if end_utc.timestamp_subsec_nanos() > 0 {
            1
        } else {
            0
        }
}

pub(crate) fn parse_duration_spec(spec: &str) -> Result<ChronoDuration> {
    if let Some(value) = spec.strip_suffix("mo") {
        let months: i64 = value.parse()?;
        return Ok(ChronoDuration::days(30 * months));
    }
    if let Some(value) = spec.strip_suffix('d') {
        let days: i64 = value.parse()?;
        return Ok(ChronoDuration::days(days));
    }
    if let Some(value) = spec.strip_suffix('h') {
        let hours: i64 = value.parse()?;
        return Ok(ChronoDuration::hours(hours));
    }
    if let Some(value) = spec.strip_suffix('m') {
        let minutes: i64 = value.parse()?;
        return Ok(ChronoDuration::minutes(minutes));
    }

    Err(anyhow::anyhow!(
        "unsupported duration specification: {spec}"
    ))
}

pub(crate) struct RangeWindow {
    pub(crate) start: DateTime<Utc>,
    pub(crate) end: DateTime<Utc>,
    pub(crate) display_end: DateTime<Utc>,
    pub(crate) duration: ChronoDuration,
}

pub(crate) fn resolve_range_window(spec: &str, tz: Tz) -> Result<RangeWindow> {
    let now = Utc::now();
    if let Some((start, raw_end)) = named_range_bounds(spec, now, tz) {
        // Clamp to "now" so charts do not render future empty buckets.
        let mut end = now.min(raw_end);
        if raw_end > now && end == now && end.timestamp_subsec_nanos() == 0 {
            end += ChronoDuration::nanoseconds(1);
        }
        let duration = end.signed_duration_since(start).max(ChronoDuration::zero());
        return Ok(RangeWindow {
            start,
            end,
            display_end: end,
            duration,
        });
    }

    let duration = parse_duration_spec(spec)?;
    let mut end = now;
    if end.timestamp_subsec_nanos() == 0 {
        end += ChronoDuration::nanoseconds(1);
    }
    let start = end - duration;
    Ok(RangeWindow {
        start,
        end,
        display_end: end,
        duration,
    })
}

pub(crate) fn named_range_bounds(
    spec: &str,
    now: DateTime<Utc>,
    tz: Tz,
) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    match spec {
        "today" => {
            let local_date = now.with_timezone(&tz).date_naive();
            let start = start_of_local_day(now, tz);
            let next_date = local_date
                .succ_opt()
                .unwrap_or(local_date + ChronoDuration::days(1));
            let end = local_midnight_utc(next_date, tz);
            Some((start, end))
        }
        "yesterday" => {
            let local_date = now.with_timezone(&tz).date_naive();
            let end = local_midnight_utc(local_date, tz);
            let previous_date = local_date
                .pred_opt()
                .unwrap_or(local_date - ChronoDuration::days(1));
            let start = local_midnight_utc(previous_date, tz);
            Some((start, end))
        }
        "thisWeek" => {
            let start = start_of_local_week(now, tz);
            // Week end must be computed via the next local boundary, not a fixed +7*24h.
            // This keeps correctness across DST transitions.
            let start_local_date = start.with_timezone(&tz).date_naive();
            let end = local_midnight_utc(start_local_date + ChronoDuration::days(7), tz);
            Some((start, end))
        }
        "thisMonth" => {
            let start = start_of_local_month(now, tz);
            Some((start, start_of_next_month(start, tz)))
        }
        _ => None,
    }
}

pub(crate) fn previous_full_days_range_bounds(
    day_count: i64,
    now: DateTime<Utc>,
    tz: Tz,
) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    if day_count <= 0 {
        return None;
    }
    let local_date = now.with_timezone(&tz).date_naive();
    let end = local_midnight_utc(local_date, tz);
    let start = local_midnight_utc(local_date - ChronoDuration::days(day_count), tz);
    Some((start, end))
}

pub(crate) fn named_range_start(spec: &str, now: DateTime<Utc>, tz: Tz) -> Option<DateTime<Utc>> {
    named_range_bounds(spec, now, tz).map(|(start, _)| start)
}

pub(crate) fn start_of_local_day(now: DateTime<Utc>, tz: Tz) -> DateTime<Utc> {
    let local = now.with_timezone(&tz);
    let date = local.date_naive();
    let naive = date
        .and_hms_opt(0, 0, 0)
        .expect("midnight should be representable");
    local_naive_to_utc(naive, tz)
}

pub(crate) fn local_midnight_utc(date: NaiveDate, tz: Tz) -> DateTime<Utc> {
    let naive = date
        .and_hms_opt(0, 0, 0)
        .expect("midnight should be representable");
    local_naive_to_utc(naive, tz)
}

pub(crate) fn start_of_local_week(now: DateTime<Utc>, tz: Tz) -> DateTime<Utc> {
    let local = now.with_timezone(&tz);
    let date = local.date_naive();
    let start_of_day = date
        .and_hms_opt(0, 0, 0)
        .expect("midnight should be representable");
    let offset_days = local.weekday().num_days_from_monday() as i64;
    local_naive_to_utc(start_of_day - ChronoDuration::days(offset_days), tz)
}

pub(crate) fn start_of_local_month(now: DateTime<Utc>, tz: Tz) -> DateTime<Utc> {
    let local = now.with_timezone(&tz);
    let date = local.date_naive();
    let first_day = date.with_day(1).unwrap_or(date);
    let naive = first_day
        .and_hms_opt(0, 0, 0)
        .expect("midnight should be representable");
    local_naive_to_utc(naive, tz)
}

pub(crate) fn start_of_next_month(start: DateTime<Utc>, tz: Tz) -> DateTime<Utc> {
    let local = start.with_timezone(&tz);
    let naive = local.naive_local();
    let mut year = naive.year();
    let mut month = naive.month();
    month += 1;
    if month > 12 {
        month = 1;
        year += 1;
    }
    let first = NaiveDate::from_ymd_opt(year, month, 1).expect("valid month");
    let naive = first
        .and_hms_opt(0, 0, 0)
        .expect("midnight should be representable");
    local_naive_to_utc(naive, tz)
}

pub(crate) fn local_naive_to_utc(naive: NaiveDateTime, tz: Tz) -> DateTime<Utc> {
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(dt) => dt.with_timezone(&Utc),
        LocalResult::Ambiguous(dt, _) => dt.with_timezone(&Utc),
        LocalResult::None => {
            // Handle nonexistent local times (e.g. DST spring-forward gaps) by
            // selecting the first valid local instant *after* the requested time.
            // This avoids silently interpreting a local timestamp as UTC.
            for step_minutes in 1..=(24 * 60) {
                let probe = naive + ChronoDuration::minutes(step_minutes);
                match tz.from_local_datetime(&probe) {
                    LocalResult::Single(dt) => return dt.with_timezone(&Utc),
                    LocalResult::Ambiguous(dt, _) => return dt.with_timezone(&Utc),
                    LocalResult::None => continue,
                }
            }
            // Extremely unlikely: no valid local instant found in the next 24h.
            naive.and_utc()
        }
    }
}

pub(crate) fn bucket_seconds_from_spec(spec: &str) -> Option<i64> {
    match spec {
        "1m" => Some(60),
        "5m" => Some(300),
        "15m" => Some(900),
        "30m" => Some(1800),
        "1h" => Some(3600),
        "6h" => Some(21_600),
        "12h" => Some(43_200),
        "1d" => Some(86_400),
        _ => None,
    }
}
