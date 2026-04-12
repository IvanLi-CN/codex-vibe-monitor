use crate::*;

const STATS_SUCCESS_LIKE_SQL: &str = "(LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'completed') OR (LOWER(TRIM(COALESCE(status, ''))) = 'http_200' AND TRIM(COALESCE(error_message, '')) = ''))";
const STATS_TERMINAL_STATUS_SQL: &str =
    "(LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending'))";
pub(crate) const INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET: &str =
    "codex_invocations_summary_rollup_v2_live_cursor";
const MISSING_INVOCATION_ARCHIVE_REPAIR_PREFIX: &str =
    "completed invocation archive is missing during summary rollup repair";

#[derive(Debug)]
pub(crate) enum SummaryWindow {
    All,
    Current(i64),
    Duration(ChronoDuration),
    Calendar(String),
}

fn stats_success_failure_select_sql() -> String {
    format!(
        "COUNT(*) AS total_count, \
         COALESCE(SUM(CASE WHEN {success_like} AND {resolved_failure} = 'none' THEN 1 ELSE 0 END), 0) AS success_count, \
         COALESCE(SUM(CASE WHEN {terminal_status} AND {resolved_failure} IN ('service_failure', 'client_failure', 'client_abort') THEN 1 ELSE 0 END), 0) AS failure_count, \
         COALESCE(SUM(cost), 0.0) AS total_cost, \
         COALESCE(SUM(total_tokens), 0) AS total_tokens",
        success_like = STATS_SUCCESS_LIKE_SQL,
        terminal_status = STATS_TERMINAL_STATUS_SQL,
        resolved_failure = crate::api::INVOCATION_RESOLVED_FAILURE_CLASS_SQL,
    )
}

fn is_missing_invocation_summary_archive_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .to_string()
            .contains(MISSING_INVOCATION_ARCHIVE_REPAIR_PREFIX)
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
    pub(crate) total_cost: f64,
    pub(crate) first_byte_sample_count: i64,
    pub(crate) first_byte_sum_ms: f64,
    pub(crate) first_byte_max_ms: f64,
    pub(crate) first_byte_histogram: String,
    pub(crate) first_response_byte_total_sample_count: i64,
    pub(crate) first_response_byte_total_sum_ms: f64,
    pub(crate) first_response_byte_total_max_ms: f64,
    pub(crate) first_response_byte_total_histogram: String,
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

pub(crate) fn normalize_non_negative_timing_value(value: Option<f64>) -> Option<f64> {
    let value = value?;
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    Some(value)
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

#[derive(Default)]
pub(crate) struct BucketAggregate {
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) in_flight_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) first_byte_ttfb_sum_ms: f64,
    pub(crate) first_byte_ttfb_values: Vec<f64>,
    pub(crate) first_byte_histogram: ApproxHistogramCounts,
    pub(crate) first_byte_sample_count: i64,
    pub(crate) first_response_byte_total_sum_ms: f64,
    pub(crate) first_response_byte_total_values: Vec<f64>,
    pub(crate) first_response_byte_total_histogram: ApproxHistogramCounts,
    pub(crate) first_response_byte_total_sample_count: i64,
}

impl BucketAggregate {
    fn validated_success_ttfb_value(status: Option<&str>, ttfb_ms: Option<f64>) -> Option<f64> {
        if status != Some("success") {
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
}

pub(crate) fn default_range() -> String {
    "1d".to_string()
}

pub(crate) fn format_naive(dt: NaiveDateTime) -> String {
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
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

pub(crate) fn bucket_spec_from_seconds(bucket_seconds: i64) -> Option<&'static str> {
    match bucket_seconds {
        60 => Some("1m"),
        300 => Some("5m"),
        900 => Some("15m"),
        1800 => Some("30m"),
        3600 => Some("1h"),
        21_600 => Some("6h"),
        43_200 => Some("12h"),
        86_400 => Some("1d"),
        _ => None,
    }
}

pub(crate) fn available_timeseries_bucket_specs(subhour_supported: bool) -> Vec<String> {
    if subhour_supported {
        vec!["1m", "5m", "15m", "30m", "1h", "6h", "12h", "1d"]
            .into_iter()
            .map(str::to_string)
            .collect()
    } else {
        vec!["1h", "6h", "12h", "1d"]
            .into_iter()
            .map(str::to_string)
            .collect()
    }
}

pub(crate) fn default_bucket_seconds(range: ChronoDuration) -> i64 {
    let seconds = range.num_seconds();
    if seconds <= 3_600 {
        60
    } else if seconds <= 172_800 {
        1_800
    } else if seconds <= 2_592_000 {
        3_600
    } else {
        86_400
    }
}

pub(crate) fn align_bucket_epoch(epoch: i64, bucket_seconds: i64, offset_seconds: i64) -> i64 {
    ((epoch + offset_seconds) / bucket_seconds) * bucket_seconds - offset_seconds
}

pub(crate) fn parse_summary_window(
    query: &SummaryQuery,
    default_limit: i64,
) -> Result<SummaryWindow> {
    match query.window.as_deref() {
        Some("current") => {
            let limit = query.limit.unwrap_or(default_limit).clamp(1, default_limit);
            Ok(SummaryWindow::Current(limit))
        }
        Some("all") => Ok(SummaryWindow::All),
        Some(raw @ ("today" | "yesterday" | "thisWeek" | "thisMonth")) => {
            Ok(SummaryWindow::Calendar(raw.to_string()))
        }
        Some(raw) => Ok(SummaryWindow::Duration(parse_duration_spec(raw)?)),
        None => Ok(SummaryWindow::Duration(ChronoDuration::days(1))),
    }
}

pub(crate) async fn query_stats_row(
    pool: &Pool<Sqlite>,
    filter: StatsFilter,
    source_scope: InvocationSourceScope,
) -> Result<StatsRow> {
    match (filter, source_scope) {
        (StatsFilter::All, InvocationSourceScope::ProxyOnly) => {
            let query = format!(
                "SELECT {} FROM codex_invocations WHERE source = ?1",
                stats_success_failure_select_sql()
            );
            sqlx::query_as::<_, StatsRow>(&query)
                .bind(SOURCE_PROXY)
                .fetch_one(pool)
                .await
                .map_err(Into::into)
        }
        (StatsFilter::All, InvocationSourceScope::All) => {
            let query = format!(
                "SELECT {} FROM codex_invocations",
                stats_success_failure_select_sql()
            );
            sqlx::query_as::<_, StatsRow>(&query)
                .fetch_one(pool)
                .await
                .map_err(Into::into)
        }
        (StatsFilter::Since(start), InvocationSourceScope::ProxyOnly) => {
            let query = format!(
                "SELECT {} FROM codex_invocations WHERE source = ?1 AND occurred_at >= ?2",
                stats_success_failure_select_sql()
            );
            sqlx::query_as::<_, StatsRow>(&query)
                .bind(SOURCE_PROXY)
                .bind(db_occurred_at_lower_bound(start))
                .fetch_one(pool)
                .await
                .map_err(Into::into)
        }
        (StatsFilter::Since(start), InvocationSourceScope::All) => {
            let query = format!(
                "SELECT {} FROM codex_invocations WHERE occurred_at >= ?1",
                stats_success_failure_select_sql()
            );
            sqlx::query_as::<_, StatsRow>(&query)
                .bind(db_occurred_at_lower_bound(start))
                .fetch_one(pool)
                .await
                .map_err(Into::into)
        }
        (StatsFilter::Range(start, end), InvocationSourceScope::ProxyOnly) => {
            let query = format!(
                "SELECT {} FROM codex_invocations WHERE source = ?1 AND occurred_at >= ?2 AND occurred_at < ?3",
                stats_success_failure_select_sql()
            );
            sqlx::query_as::<_, StatsRow>(&query)
                .bind(SOURCE_PROXY)
                .bind(db_occurred_at_lower_bound(start))
                .bind(db_occurred_at_upper_bound(end))
                .fetch_one(pool)
                .await
                .map_err(Into::into)
        }
        (StatsFilter::Range(start, end), InvocationSourceScope::All) => {
            let query = format!(
                "SELECT {} FROM codex_invocations WHERE occurred_at >= ?1 AND occurred_at < ?2",
                stats_success_failure_select_sql()
            );
            sqlx::query_as::<_, StatsRow>(&query)
                .bind(db_occurred_at_lower_bound(start))
                .bind(db_occurred_at_upper_bound(end))
                .fetch_one(pool)
                .await
                .map_err(Into::into)
        }
        (StatsFilter::RecentLimit(limit), InvocationSourceScope::ProxyOnly) => {
            let query = format!(
                "WITH recent AS ( \
                    SELECT * \
                    FROM codex_invocations \
                    WHERE source = ?1 \
                    ORDER BY occurred_at DESC \
                    LIMIT ?2 \
                ) \
                SELECT {} FROM recent",
                stats_success_failure_select_sql()
            );
            sqlx::query_as::<_, StatsRow>(&query)
                .bind(SOURCE_PROXY)
                .bind(limit)
                .fetch_one(pool)
                .await
                .map_err(Into::into)
        }
        (StatsFilter::RecentLimit(limit), InvocationSourceScope::All) => {
            let query = format!(
                "WITH recent AS ( \
                    SELECT * \
                    FROM codex_invocations \
                    ORDER BY occurred_at DESC \
                    LIMIT ?1 \
                ) \
                SELECT {} FROM recent",
                stats_success_failure_select_sql()
            );
            sqlx::query_as::<_, StatsRow>(&query)
                .bind(limit)
                .fetch_one(pool)
                .await
                .map_err(Into::into)
        }
    }
}

#[derive(Debug, FromRow)]
struct ArchiveBatchPathRow {
    file_path: String,
    historical_rollups_materialized_at: Option<String>,
    needs_overall: Option<i64>,
    needs_failures: Option<i64>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct ArchivedInvocationFailureRow {
    pub(crate) occurred_at: String,
    pub(crate) source: String,
    pub(crate) status: Option<String>,
    pub(crate) error_message: Option<String>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) failure_class: Option<String>,
    pub(crate) is_actionable: Option<i64>,
}

#[derive(Debug, Default)]
struct ClearedSummaryRollupBuckets {
    overall: HashSet<(i64, String)>,
    failures: HashSet<(i64, String)>,
}

impl ClearedSummaryRollupBuckets {
    fn targets_to_clear_for_bucket(
        &mut self,
        key: &(i64, String),
        requested_targets: &[&str],
    ) -> Vec<&'static str> {
        let mut targets = Vec::new();
        if requested_targets.contains(&HOURLY_ROLLUP_TARGET_INVOCATIONS)
            && self.overall.insert(key.clone())
        {
            targets.push(HOURLY_ROLLUP_TARGET_INVOCATIONS);
        }
        if requested_targets.contains(&HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES)
            && self.failures.insert(key.clone())
        {
            targets.push(HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES);
        }
        targets
    }
}

const INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DATASET: &str = "codex_invocations_summary_rollup_v2";
const INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DONE: i64 = 1;
const INVOCATION_SUMMARY_ROLLUP_TARGETS: [&str; 2] = [
    HOURLY_ROLLUP_TARGET_INVOCATIONS,
    HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
];

async fn load_invocation_hourly_source_rows_after_id(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    start_after_id: i64,
    source_scope: InvocationSourceScope,
    limit: i64,
) -> Result<Vec<InvocationHourlySourceRecord>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            id,
            occurred_at,
            source,
            status,
            detail_level,
            total_tokens,
            cost,
            error_message,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
        FROM codex_invocations
        WHERE id >
        "#,
    );
    query.push_bind(start_after_id);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" ORDER BY id ASC LIMIT ").push_bind(limit);
    query
        .build_query_as::<InvocationHourlySourceRecord>()
        .fetch_all(executor)
        .await
        .map_err(Into::into)
}

async fn load_completed_invocation_archive_paths(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
) -> Result<Vec<ArchiveBatchPathRow>> {
    sqlx::query_as::<_, ArchiveBatchPathRow>(
        r#"
        SELECT
            file_path,
            historical_rollups_materialized_at,
            NULL AS needs_overall,
            NULL AS needs_failures
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
        ORDER BY month_key ASC, created_at ASC, id ASC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_all(executor)
    .await
    .map_err(Into::into)
}

async fn load_invocation_archives_missing_rollup_target(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    target: &str,
) -> Result<Vec<ArchiveBatchPathRow>> {
    sqlx::query_as::<_, ArchiveBatchPathRow>(
        r#"
        SELECT
            batches.file_path,
            batches.historical_rollups_materialized_at,
            NULL AS needs_overall,
            NULL AS needs_failures
        FROM archive_batches AS batches
        WHERE batches.dataset = 'codex_invocations'
          AND batches.status = ?1
          AND NOT EXISTS(
            SELECT 1
            FROM hourly_rollup_archive_replay AS replay
            WHERE replay.target = ?2
              AND replay.dataset = 'codex_invocations'
              AND replay.file_path = batches.file_path
          )
        ORDER BY batches.month_key ASC, batches.created_at ASC, batches.id ASC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(target)
    .fetch_all(executor)
    .await
    .map_err(Into::into)
}

async fn load_invocation_archives_missing_summary_rollup_markers(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
) -> Result<Vec<ArchiveBatchPathRow>> {
    sqlx::query_as::<_, ArchiveBatchPathRow>(
        r#"
        SELECT
            batches.file_path,
            batches.historical_rollups_materialized_at,
            CASE
                WHEN EXISTS(
                    SELECT 1
                    FROM hourly_rollup_archive_replay AS replay
                    WHERE replay.target = ?2
                      AND replay.dataset = 'codex_invocations'
                      AND replay.file_path = batches.file_path
                ) THEN 0
                ELSE 1
            END AS needs_overall,
            CASE
                WHEN EXISTS(
                    SELECT 1
                    FROM hourly_rollup_archive_replay AS replay
                    WHERE replay.target = ?3
                      AND replay.dataset = 'codex_invocations'
                      AND replay.file_path = batches.file_path
                ) THEN 0
                ELSE 1
            END AS needs_failures
        FROM archive_batches AS batches
        WHERE batches.dataset = 'codex_invocations'
          AND batches.status = ?1
          AND (
            NOT EXISTS(
                SELECT 1
                FROM hourly_rollup_archive_replay AS replay
                WHERE replay.target = ?2
                  AND replay.dataset = 'codex_invocations'
                  AND replay.file_path = batches.file_path
            )
            OR NOT EXISTS(
                SELECT 1
                FROM hourly_rollup_archive_replay AS replay
                WHERE replay.target = ?3
                  AND replay.dataset = 'codex_invocations'
                  AND replay.file_path = batches.file_path
            )
          )
        ORDER BY batches.month_key ASC, batches.created_at ASC, batches.id ASC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES)
    .fetch_all(executor)
    .await
    .map_err(Into::into)
}

async fn open_invocation_archive_batch_pool(
    archive_row: &ArchiveBatchPathRow,
    read_surface: &'static str,
) -> Result<Option<(Pool<Sqlite>, TempSqliteCleanup)>> {
    let archive_path = PathBuf::from(&archive_row.file_path);
    let is_materialized_archive = archive_row.historical_rollups_materialized_at.is_some();
    if !archive_path.exists() {
        warn!(
            file_path = archive_row.file_path,
            read_surface,
            historical_rollups_materialized = is_materialized_archive,
            "skipping missing invocation archive while serving read-only historical fallback"
        );
        return Ok(None);
    }

    let temp_path = PathBuf::from(format!(
        "{}.{}.sqlite",
        archive_path.display(),
        retention_temp_suffix()
    ));
    if temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }
    let temp_cleanup = TempSqliteCleanup(temp_path.clone());
    if let Err(err) = inflate_gzip_sqlite_file(&archive_path, &temp_path) {
        drop(temp_cleanup);
        if is_materialized_archive {
            warn!(
                file_path = archive_row.file_path,
                read_surface,
                error = %err,
                "skipping unreadable materialized invocation archive while serving read-only historical fallback"
            );
            return Ok(None);
        }
        return Err(err);
    }
    let archive_pool = match SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_for_path(&temp_path))
        .await
        .with_context(|| format!("failed to open archive batch {}", archive_path.display()))
    {
        Ok(pool) => pool,
        Err(err) => {
            drop(temp_cleanup);
            if is_materialized_archive {
                warn!(
                    file_path = archive_row.file_path,
                    read_surface,
                    error = %err,
                    "skipping unreadable materialized invocation archive while serving read-only historical fallback"
                );
                return Ok(None);
            }
            return Err(err);
        }
    };

    Ok(Some((archive_pool, temp_cleanup)))
}

#[derive(Debug, Clone, FromRow)]
struct MaterializedBucketRow {
    bucket_start_epoch: i64,
    source: String,
}

async fn load_materialized_rollup_bucket_sources(
    pool: &Pool<Sqlite>,
    target: &str,
    bucket_sources: &HashSet<(i64, String)>,
) -> Result<HashSet<(i64, String)>> {
    if bucket_sources.is_empty() {
        return Ok(HashSet::new());
    }

    let min_bucket_epoch = bucket_sources
        .iter()
        .map(|(bucket_start_epoch, _)| *bucket_start_epoch)
        .min()
        .ok_or_else(|| anyhow!("missing minimum materialized bucket epoch"))?;
    let max_bucket_epoch = bucket_sources
        .iter()
        .map(|(bucket_start_epoch, _)| *bucket_start_epoch)
        .max()
        .ok_or_else(|| anyhow!("missing maximum materialized bucket epoch"))?;

    let rows = sqlx::query_as::<_, MaterializedBucketRow>(
        r#"
        SELECT bucket_start_epoch, source
        FROM hourly_rollup_materialized_buckets
        WHERE target = ?1
          AND bucket_start_epoch >= ?2
          AND bucket_start_epoch <= ?3
        "#,
    )
    .bind(target)
    .bind(min_bucket_epoch)
    .bind(max_bucket_epoch)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| (row.bucket_start_epoch, row.source))
        .filter(|key| bucket_sources.contains(key))
        .collect())
}

fn shanghai_month_keys_for_bucket_starts(
    bucket_start_epochs: impl IntoIterator<Item = i64>,
) -> HashSet<String> {
    bucket_start_epochs
        .into_iter()
        .filter_map(|bucket_start_epoch| {
            Utc.timestamp_opt(bucket_start_epoch, 0)
                .single()
                .map(|dt| dt.with_timezone(&Shanghai).format("%Y-%m").to_string())
        })
        .collect()
}

async fn load_replayed_invocation_archive_paths_for_month_keys(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    target: &str,
    month_keys: &HashSet<String>,
) -> Result<Vec<String>> {
    if month_keys.is_empty() {
        return Ok(Vec::new());
    }

    let mut month_keys = month_keys.iter().cloned().collect::<Vec<_>>();
    month_keys.sort();

    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT file_path
        FROM archive_batches AS batches
        WHERE batches.dataset = 'codex_invocations'
          AND batches.status =
        "#,
    );
    query.push_bind(ARCHIVE_STATUS_COMPLETED);
    query.push(
        r#"
         AND EXISTS(
            SELECT 1
            FROM hourly_rollup_archive_replay AS replay
            WHERE replay.target =
        "#,
    );
    query.push_bind(target);
    query.push(
        r#"
              AND replay.dataset = 'codex_invocations'
              AND replay.file_path = batches.file_path
         )
         AND batches.month_key IN (
        "#,
    );
    {
        let mut separated = query.separated(", ");
        for month_key in month_keys {
            separated.push_bind(month_key);
        }
    }
    query.push(") ORDER BY batches.month_key ASC, batches.created_at ASC, batches.id ASC");

    query
        .build_query_scalar::<String>()
        .fetch_all(executor)
        .await
        .map_err(Into::into)
}

fn materialized_archive_path_row(file_path: String) -> ArchiveBatchPathRow {
    ArchiveBatchPathRow {
        file_path,
        historical_rollups_materialized_at: Some("materialized".to_string()),
        needs_overall: None,
        needs_failures: None,
    }
}

async fn load_materialized_invocation_archive_hourly_rollup_deltas_for_bucket_sources(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    bucket_sources: &HashSet<(i64, String)>,
) -> Result<BTreeMap<(i64, String), InvocationHourlyRollupDelta>> {
    if bucket_sources.is_empty() {
        return Ok(BTreeMap::new());
    }

    let month_keys = shanghai_month_keys_for_bucket_starts(
        bucket_sources
            .iter()
            .map(|(bucket_start_epoch, _)| *bucket_start_epoch),
    );
    let archive_paths = load_replayed_invocation_archive_paths_for_month_keys(
        pool,
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        &month_keys,
    )
    .await?;
    let mut materialized = BTreeMap::<(i64, String), InvocationHourlyRollupDelta>::new();

    for archive_path in archive_paths {
        let archive_row = materialized_archive_path_row(archive_path);
        let Some((archive_pool, temp_cleanup)) =
            open_invocation_archive_batch_pool(&archive_row, "stats-summary").await?
        else {
            continue;
        };
        let mut cursor_id = 0_i64;
        loop {
            let rows = load_invocation_hourly_source_rows_after_id(
                &archive_pool,
                cursor_id,
                source_scope,
                BACKFILL_BATCH_SIZE,
            )
            .await?;
            if rows.is_empty() {
                break;
            }
            cursor_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
            let filtered_rows = rows
                .into_iter()
                .filter(|row| invocation_hourly_source_record_matches_range(row, range))
                .filter_map(|row| {
                    summary_rollup_bucket_start_epoch(&row.occurred_at)
                        .ok()
                        .map(|bucket_start_epoch| ((bucket_start_epoch, row.source.clone()), row))
                })
                .filter(|(key, _)| bucket_sources.contains(key))
                .map(|(_, row)| row)
                .collect::<Vec<_>>();
            if filtered_rows.is_empty() {
                continue;
            }
            accumulate_invocation_hourly_overall_rollups(&mut materialized, &filtered_rows)?;
        }

        archive_pool.close().await;
        drop(temp_cleanup);
    }

    Ok(materialized)
}

fn subtract_approx_histogram_counts(
    archive_histogram: &[i64],
    materialized_histogram: &[i64],
) -> ApproxHistogramCounts {
    let expected_len = archive_histogram.len().max(materialized_histogram.len());
    let mut delta = archive_histogram.to_vec();
    delta.resize(expected_len, 0);
    for (idx, slot) in delta.iter_mut().enumerate() {
        let materialized = materialized_histogram.get(idx).copied().unwrap_or_default();
        *slot = slot.saturating_sub(materialized).max(0);
    }
    delta
}

fn subtract_nonnegative_i64(archive_value: i64, materialized_value: i64) -> i64 {
    archive_value.saturating_sub(materialized_value).max(0)
}

fn subtract_nonnegative_f64(archive_value: f64, materialized_value: f64) -> f64 {
    (archive_value - materialized_value).max(0.0)
}

async fn load_materialized_invocation_rollup_record(
    pool: &Pool<Sqlite>,
    bucket_start_epoch: i64,
    source: &str,
) -> Result<Option<InvocationHourlyRollupRecord>> {
    sqlx::query_as::<_, InvocationHourlyRollupRecord>(
        r#"
        SELECT
            bucket_start_epoch,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram,
            first_response_byte_total_sample_count,
            first_response_byte_total_sum_ms,
            first_response_byte_total_max_ms,
            first_response_byte_total_histogram
        FROM invocation_rollup_hourly
        WHERE bucket_start_epoch = ?1
          AND source = ?2
        LIMIT 1
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(source)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

fn build_invocation_hourly_rollup_delta_record(
    bucket_start_epoch: i64,
    archive_delta: &InvocationHourlyRollupDelta,
    materialized_row: Option<&InvocationHourlyRollupRecord>,
) -> Result<Option<InvocationHourlyRollupRecord>> {
    let total_count = subtract_nonnegative_i64(
        archive_delta.total_count,
        materialized_row
            .map(|row| row.total_count.max(0))
            .unwrap_or(0),
    );
    let success_count = subtract_nonnegative_i64(
        archive_delta.success_count,
        materialized_row
            .map(|row| row.success_count.max(0))
            .unwrap_or(0),
    );
    let failure_count = subtract_nonnegative_i64(
        archive_delta.failure_count,
        materialized_row
            .map(|row| row.failure_count.max(0))
            .unwrap_or(0),
    );
    let total_tokens = subtract_nonnegative_i64(
        archive_delta.total_tokens,
        materialized_row
            .map(|row| row.total_tokens.max(0))
            .unwrap_or(0),
    );
    let total_cost = subtract_nonnegative_f64(
        archive_delta.total_cost,
        materialized_row.map(|row| row.total_cost).unwrap_or(0.0),
    );

    let first_byte_histogram = subtract_approx_histogram_counts(
        &archive_delta.first_byte_histogram,
        &materialized_row
            .map(|row| decode_approx_histogram(&row.first_byte_histogram))
            .unwrap_or_else(empty_approx_histogram),
    );
    let first_byte_sample_count = first_byte_histogram.iter().copied().sum::<i64>();
    let first_byte_sum_ms = (archive_delta.first_byte_sum_ms
        - materialized_row
            .map(|row| row.first_byte_sum_ms)
            .unwrap_or(0.0))
    .max(0.0);

    let first_response_byte_total_histogram = subtract_approx_histogram_counts(
        &archive_delta.first_response_byte_total_histogram,
        &materialized_row
            .map(|row| decode_approx_histogram(&row.first_response_byte_total_histogram))
            .unwrap_or_else(empty_approx_histogram),
    );
    let first_response_byte_total_sample_count = first_response_byte_total_histogram
        .iter()
        .copied()
        .sum::<i64>();
    let first_response_byte_total_sum_ms = (archive_delta.first_response_byte_total_sum_ms
        - materialized_row
            .map(|row| row.first_response_byte_total_sum_ms)
            .unwrap_or(0.0))
    .max(0.0);

    if total_count <= 0
        && success_count <= 0
        && failure_count <= 0
        && total_tokens <= 0
        && total_cost <= 0.0
        && first_byte_sample_count <= 0
        && first_response_byte_total_sample_count <= 0
    {
        return Ok(None);
    }

    Ok(Some(InvocationHourlyRollupRecord {
        bucket_start_epoch,
        total_count,
        success_count,
        failure_count,
        total_tokens,
        total_cost,
        first_byte_sample_count,
        first_byte_sum_ms,
        first_byte_max_ms: if first_byte_sample_count > 0 {
            approx_histogram_percentile_ms(&first_byte_histogram, 1.0).unwrap_or(0.0)
        } else {
            0.0
        },
        first_byte_histogram: encode_approx_histogram(&first_byte_histogram)?,
        first_response_byte_total_sample_count,
        first_response_byte_total_sum_ms,
        first_response_byte_total_max_ms: if first_response_byte_total_sample_count > 0 {
            approx_histogram_percentile_ms(&first_response_byte_total_histogram, 1.0).unwrap_or(0.0)
        } else {
            0.0
        },
        first_response_byte_total_histogram: encode_approx_histogram(
            &first_response_byte_total_histogram,
        )?,
    }))
}

fn build_materialized_pending_invocation_rollup_overlap_record(
    bucket_start_epoch: i64,
    materialized_row: Option<&InvocationHourlyRollupRecord>,
    completed_archive_delta: Option<&InvocationHourlyRollupDelta>,
) -> Result<Option<InvocationHourlyRollupRecord>> {
    let Some(materialized_row) = materialized_row else {
        return Ok(None);
    };

    let total_count = subtract_nonnegative_i64(
        materialized_row.total_count.max(0),
        completed_archive_delta
            .map(|delta| delta.total_count.max(0))
            .unwrap_or(0),
    );
    let success_count = subtract_nonnegative_i64(
        materialized_row.success_count.max(0),
        completed_archive_delta
            .map(|delta| delta.success_count.max(0))
            .unwrap_or(0),
    );
    let failure_count = subtract_nonnegative_i64(
        materialized_row.failure_count.max(0),
        completed_archive_delta
            .map(|delta| delta.failure_count.max(0))
            .unwrap_or(0),
    );
    let total_tokens = subtract_nonnegative_i64(
        materialized_row.total_tokens.max(0),
        completed_archive_delta
            .map(|delta| delta.total_tokens.max(0))
            .unwrap_or(0),
    );
    let total_cost = subtract_nonnegative_f64(
        materialized_row.total_cost,
        completed_archive_delta
            .map(|delta| delta.total_cost)
            .unwrap_or(0.0),
    );

    let materialized_first_byte_histogram =
        decode_approx_histogram(&materialized_row.first_byte_histogram);
    let empty_first_byte_histogram = empty_approx_histogram();
    let first_byte_histogram = subtract_approx_histogram_counts(
        &materialized_first_byte_histogram,
        completed_archive_delta
            .map(|delta| delta.first_byte_histogram.as_slice())
            .unwrap_or(empty_first_byte_histogram.as_slice()),
    );
    let first_byte_sample_count = first_byte_histogram.iter().copied().sum::<i64>();
    let first_byte_sum_ms = subtract_nonnegative_f64(
        materialized_row.first_byte_sum_ms,
        completed_archive_delta
            .map(|delta| delta.first_byte_sum_ms)
            .unwrap_or(0.0),
    );

    let materialized_first_response_byte_total_histogram =
        decode_approx_histogram(&materialized_row.first_response_byte_total_histogram);
    let empty_first_response_byte_total_histogram = empty_approx_histogram();
    let first_response_byte_total_histogram = subtract_approx_histogram_counts(
        &materialized_first_response_byte_total_histogram,
        completed_archive_delta
            .map(|delta| delta.first_response_byte_total_histogram.as_slice())
            .unwrap_or(empty_first_response_byte_total_histogram.as_slice()),
    );
    let first_response_byte_total_sample_count = first_response_byte_total_histogram
        .iter()
        .copied()
        .sum::<i64>();
    let first_response_byte_total_sum_ms = subtract_nonnegative_f64(
        materialized_row.first_response_byte_total_sum_ms,
        completed_archive_delta
            .map(|delta| delta.first_response_byte_total_sum_ms)
            .unwrap_or(0.0),
    );

    if total_count <= 0
        && success_count <= 0
        && failure_count <= 0
        && total_tokens <= 0
        && total_cost <= 0.0
        && first_byte_sample_count <= 0
        && first_response_byte_total_sample_count <= 0
    {
        return Ok(None);
    }

    Ok(Some(InvocationHourlyRollupRecord {
        bucket_start_epoch,
        total_count,
        success_count,
        failure_count,
        total_tokens,
        total_cost,
        first_byte_sample_count,
        first_byte_sum_ms,
        first_byte_max_ms: if first_byte_sample_count > 0 {
            approx_histogram_percentile_ms(&first_byte_histogram, 1.0).unwrap_or(0.0)
        } else {
            0.0
        },
        first_byte_histogram: encode_approx_histogram(&first_byte_histogram)?,
        first_response_byte_total_sample_count,
        first_response_byte_total_sum_ms,
        first_response_byte_total_max_ms: if first_response_byte_total_sample_count > 0 {
            approx_histogram_percentile_ms(&first_response_byte_total_histogram, 1.0).unwrap_or(0.0)
        } else {
            0.0
        },
        first_response_byte_total_histogram: encode_approx_histogram(
            &first_response_byte_total_histogram,
        )?,
    }))
}

pub(crate) async fn query_unmaterialized_invocation_archive_hourly_rollup_deltas(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
) -> Result<Vec<InvocationHourlyRollupRecord>> {
    let pending_overall =
        load_pending_invocation_archive_hourly_rollup_deltas(pool, source_scope, range).await?;
    let materialized_bucket_sources = load_materialized_rollup_bucket_sources(
        pool,
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        &pending_overall.keys().cloned().collect::<HashSet<_>>(),
    )
    .await?;
    let materialized_overall =
        load_materialized_invocation_archive_hourly_rollup_deltas_for_bucket_sources(
            pool,
            source_scope,
            range,
            &materialized_bucket_sources,
        )
        .await?;
    let mut delta_rows = Vec::new();
    for ((bucket_start_epoch, source), archive_delta) in pending_overall {
        let materialized_overlap = build_materialized_pending_invocation_rollup_overlap_record(
            bucket_start_epoch,
            load_materialized_invocation_rollup_record(pool, bucket_start_epoch, &source)
                .await?
                .as_ref(),
            materialized_bucket_sources
                .contains(&(bucket_start_epoch, source.clone()))
                .then(|| materialized_overall.get(&(bucket_start_epoch, source.clone())))
                .flatten(),
        )?;
        if let Some(delta_row) = build_invocation_hourly_rollup_delta_record(
            bucket_start_epoch,
            &archive_delta,
            materialized_overlap.as_ref(),
        )? {
            delta_rows.push(delta_row);
        }
    }

    delta_rows.sort_by_key(|row| row.bucket_start_epoch);
    Ok(delta_rows)
}

async fn load_pending_invocation_archive_hourly_rollup_deltas(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
) -> Result<BTreeMap<(i64, String), InvocationHourlyRollupDelta>> {
    let archive_rows =
        load_invocation_archives_missing_rollup_target(pool, HOURLY_ROLLUP_TARGET_INVOCATIONS)
            .await?;
    let mut pending = BTreeMap::<(i64, String), InvocationHourlyRollupDelta>::new();

    for archive_row in archive_rows {
        let Some((archive_pool, temp_cleanup)) =
            open_invocation_archive_batch_pool(&archive_row, "stats-summary").await?
        else {
            continue;
        };
        let mut cursor_id = 0_i64;
        loop {
            let rows = load_invocation_hourly_source_rows_after_id(
                &archive_pool,
                cursor_id,
                source_scope,
                BACKFILL_BATCH_SIZE,
            )
            .await?;
            if rows.is_empty() {
                break;
            }
            cursor_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
            let filtered_rows = rows
                .into_iter()
                .filter(|row| invocation_hourly_source_record_matches_range(row, range))
                .collect::<Vec<_>>();
            if filtered_rows.is_empty() {
                continue;
            }
            accumulate_invocation_hourly_overall_rollups(&mut pending, &filtered_rows)?;
        }

        archive_pool.close().await;
        drop(temp_cleanup);
    }

    Ok(pending)
}

fn invocation_hourly_source_record_matches_range(
    row: &InvocationHourlySourceRecord,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
) -> bool {
    let Some((start, end)) = range else {
        return true;
    };
    let Some(occurred_at_utc) = parse_to_utc_datetime(&row.occurred_at) else {
        return false;
    };
    occurred_at_utc >= start && occurred_at_utc < end
}

fn archived_failure_rollup_key(
    row: &ArchivedInvocationFailureRow,
) -> Result<Option<(i64, String, String, i64, String)>> {
    let classification = resolve_failure_classification(
        row.status.as_deref(),
        row.error_message.as_deref(),
        row.failure_kind.as_deref(),
        row.failure_class.as_deref(),
        row.is_actionable,
    );
    if classification.failure_class == FailureClass::None {
        return Ok(None);
    }
    Ok(Some((
        summary_rollup_bucket_start_epoch(&row.occurred_at)?,
        row.source.clone(),
        classification.failure_class.as_str().to_string(),
        classification.is_actionable as i64,
        categorize_error(row.error_message.as_deref().unwrap_or_default()),
    )))
}

async fn load_missing_failure_rollup_row_counts_for_rows(
    pool: &Pool<Sqlite>,
    rows: &[ArchivedInvocationFailureRow],
    materialized_bucket_sources: &HashSet<(i64, String)>,
    completed_archive_row_counts: &HashMap<(i64, String, String, i64, String), usize>,
) -> Result<HashMap<(i64, String, String, i64, String), usize>> {
    let mut grouped_counts = HashMap::<(i64, String, String, i64, String), usize>::new();
    for row in rows {
        let Some(key) = archived_failure_rollup_key(row)? else {
            continue;
        };
        *grouped_counts.entry(key).or_default() += 1;
    }
    if grouped_counts.is_empty() {
        return Ok(HashMap::new());
    }

    let mut missing_counts = HashMap::new();
    for (
        (bucket_start_epoch, source, failure_class, is_actionable, error_category),
        archive_count,
    ) in grouped_counts
    {
        let bucket_source = (bucket_start_epoch, source.clone());
        let materialized_count = sqlx::query_scalar::<_, i64>(
            "SELECT failure_count FROM invocation_failure_rollup_hourly WHERE bucket_start_epoch = ?1 AND source = ?2 AND failure_class = ?3 AND is_actionable = ?4 AND error_category = ?5 LIMIT 1",
        )
        .bind(bucket_start_epoch)
        .bind(&source)
        .bind(&failure_class)
        .bind(is_actionable)
        .bind(&error_category)
        .fetch_optional(pool)
        .await?
        .unwrap_or_default()
        .max(0) as usize;
        let pending_batch_overlap = if materialized_bucket_sources.contains(&bucket_source) {
            materialized_count.saturating_sub(
                completed_archive_row_counts
                    .get(&(
                        bucket_start_epoch,
                        source.clone(),
                        failure_class.clone(),
                        is_actionable,
                        error_category.clone(),
                    ))
                    .copied()
                    .unwrap_or_default(),
            )
        } else {
            materialized_count
        };
        let missing_count = archive_count.saturating_sub(pending_batch_overlap);
        if missing_count > 0 {
            missing_counts.insert(
                (
                    bucket_start_epoch,
                    source,
                    failure_class,
                    is_actionable,
                    error_category,
                ),
                missing_count,
            );
        }
    }

    Ok(missing_counts)
}

fn add_invocation_hourly_source_record_to_proxy_perf_rollups(
    perf: &mut BTreeMap<(i64, String), ProxyPerfStageHourlyDelta>,
    row: &InvocationHourlySourceRecord,
) -> Result<()> {
    if row.source != SOURCE_PROXY {
        return Ok(());
    }
    let bucket_start_epoch = summary_rollup_bucket_start_epoch(&row.occurred_at)?;
    for (stage, value_ms) in [
        ("total", row.t_total_ms),
        ("requestRead", row.t_req_read_ms),
        ("requestParse", row.t_req_parse_ms),
        ("upstreamConnect", row.t_upstream_connect_ms),
        ("upstreamFirstByte", row.t_upstream_ttfb_ms),
        ("upstreamStream", row.t_upstream_stream_ms),
        ("responseParse", row.t_resp_parse_ms),
        ("persistence", row.t_persist_ms),
    ] {
        record_proxy_perf_stage_sample(perf, bucket_start_epoch, stage, value_ms);
    }
    Ok(())
}

async fn load_materialized_proxy_perf_rollups_for_range(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<BTreeMap<(i64, String), ProxyPerfStageHourlyRollupRecord>> {
    Ok(
        query_proxy_perf_stage_hourly_rollup_range(pool, start.timestamp(), end.timestamp())
            .await?
            .into_iter()
            .map(|row| ((row.bucket_start_epoch, row.stage.clone()), row))
            .collect(),
    )
}

fn build_proxy_perf_stage_rollup_delta(
    archive_delta: &ProxyPerfStageHourlyDelta,
    materialized_row: Option<&ProxyPerfStageHourlyRollupRecord>,
) -> Result<Option<ProxyPerfStageHourlyDelta>> {
    let histogram = subtract_approx_histogram_counts(
        &archive_delta.histogram,
        &materialized_row
            .map(|row| decode_approx_histogram(&row.histogram))
            .unwrap_or_else(empty_approx_histogram),
    );
    let sample_count = histogram.iter().copied().sum::<i64>();
    let sum_ms = subtract_nonnegative_f64(
        archive_delta.sum_ms,
        materialized_row.map(|row| row.sum_ms).unwrap_or(0.0),
    );

    if sample_count <= 0 && sum_ms <= 0.0 {
        return Ok(None);
    }

    Ok(Some(ProxyPerfStageHourlyDelta {
        sample_count,
        sum_ms,
        max_ms: if sample_count > 0 {
            approx_histogram_percentile_ms(&histogram, 1.0).unwrap_or(0.0)
        } else {
            0.0
        },
        histogram,
    }))
}

fn build_materialized_pending_proxy_perf_overlap(
    materialized_row: Option<&ProxyPerfStageHourlyRollupRecord>,
    completed_archive_delta: Option<&ProxyPerfStageHourlyDelta>,
) -> Option<ProxyPerfStageHourlyRollupRecord> {
    let materialized_row = materialized_row?;
    let materialized_histogram = decode_approx_histogram(&materialized_row.histogram);
    let empty_histogram = empty_approx_histogram();
    let histogram = subtract_approx_histogram_counts(
        &materialized_histogram,
        completed_archive_delta
            .map(|delta| delta.histogram.as_slice())
            .unwrap_or(empty_histogram.as_slice()),
    );
    let sample_count = histogram.iter().copied().sum::<i64>();
    let sum_ms = subtract_nonnegative_f64(
        materialized_row.sum_ms,
        completed_archive_delta
            .map(|delta| delta.sum_ms)
            .unwrap_or(0.0),
    );
    if sample_count <= 0 && sum_ms <= 0.0 {
        return None;
    }

    Some(ProxyPerfStageHourlyRollupRecord {
        bucket_start_epoch: materialized_row.bucket_start_epoch,
        stage: materialized_row.stage.clone(),
        sample_count,
        sum_ms,
        max_ms: if sample_count > 0 {
            approx_histogram_percentile_ms(&histogram, 1.0).unwrap_or(0.0)
        } else {
            0.0
        },
        histogram: encode_approx_histogram(&histogram).unwrap_or_default(),
    })
}

async fn load_proxy_perf_stage_rollups_by_materialization_state(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<BTreeMap<(i64, String), ProxyPerfStageHourlyDelta>> {
    let archive_rows =
        load_invocation_archives_missing_rollup_target(pool, HOURLY_ROLLUP_TARGET_PROXY_PERF)
            .await?;
    let mut pending = BTreeMap::<(i64, String), ProxyPerfStageHourlyDelta>::new();

    for archive_row in archive_rows {
        let Some((archive_pool, temp_cleanup)) =
            open_invocation_archive_batch_pool(&archive_row, "proxy-perf").await?
        else {
            continue;
        };
        let mut cursor_id = 0_i64;
        loop {
            let rows = load_invocation_hourly_source_rows_after_id(
                &archive_pool,
                cursor_id,
                InvocationSourceScope::ProxyOnly,
                BACKFILL_BATCH_SIZE,
            )
            .await?;
            if rows.is_empty() {
                break;
            }
            cursor_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
            for row in rows {
                if !invocation_hourly_source_record_matches_range(&row, Some((start, end))) {
                    continue;
                }
                add_invocation_hourly_source_record_to_proxy_perf_rollups(&mut pending, &row)?;
            }
        }

        archive_pool.close().await;
        drop(temp_cleanup);
    }

    Ok(pending)
}

async fn load_materialized_proxy_perf_stage_rollups_for_buckets(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    bucket_start_epochs: &HashSet<i64>,
) -> Result<BTreeMap<(i64, String), ProxyPerfStageHourlyDelta>> {
    if bucket_start_epochs.is_empty() {
        return Ok(BTreeMap::new());
    }

    let month_keys = shanghai_month_keys_for_bucket_starts(bucket_start_epochs.iter().copied());
    let archive_paths = load_replayed_invocation_archive_paths_for_month_keys(
        pool,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
        &month_keys,
    )
    .await?;
    let mut materialized = BTreeMap::<(i64, String), ProxyPerfStageHourlyDelta>::new();

    for archive_path in archive_paths {
        let archive_row = materialized_archive_path_row(archive_path);
        let Some((archive_pool, temp_cleanup)) =
            open_invocation_archive_batch_pool(&archive_row, "proxy-perf").await?
        else {
            continue;
        };
        let mut cursor_id = 0_i64;
        loop {
            let rows = load_invocation_hourly_source_rows_after_id(
                &archive_pool,
                cursor_id,
                InvocationSourceScope::ProxyOnly,
                BACKFILL_BATCH_SIZE,
            )
            .await?;
            if rows.is_empty() {
                break;
            }
            cursor_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
            for row in rows {
                if !invocation_hourly_source_record_matches_range(&row, Some((start, end))) {
                    continue;
                }
                let Ok(bucket_start_epoch) = summary_rollup_bucket_start_epoch(&row.occurred_at)
                else {
                    continue;
                };
                if !bucket_start_epochs.contains(&bucket_start_epoch) {
                    continue;
                }
                add_invocation_hourly_source_record_to_proxy_perf_rollups(&mut materialized, &row)?;
            }
        }

        archive_pool.close().await;
        drop(temp_cleanup);
    }

    Ok(materialized)
}

pub(crate) async fn query_unmaterialized_proxy_perf_stage_rollups_from_archives(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<BTreeMap<String, ProxyPerfStageHourlyDelta>> {
    let archive_perf_by_bucket_stage =
        load_proxy_perf_stage_rollups_by_materialization_state(pool, start, end).await?;

    if archive_perf_by_bucket_stage.is_empty() {
        return Ok(BTreeMap::new());
    }

    let materialized_bucket_sources = load_materialized_rollup_bucket_sources(
        pool,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
        &archive_perf_by_bucket_stage
            .keys()
            .map(|(bucket_start_epoch, _)| (*bucket_start_epoch, SOURCE_PROXY.to_string()))
            .collect::<HashSet<_>>(),
    )
    .await?;
    let materialized_archive_perf_by_bucket_stage =
        load_materialized_proxy_perf_stage_rollups_for_buckets(
            pool,
            start,
            end,
            &materialized_bucket_sources
                .iter()
                .map(|(bucket_start_epoch, _)| *bucket_start_epoch)
                .collect::<HashSet<_>>(),
        )
        .await?;
    let materialized_by_bucket_stage =
        load_materialized_proxy_perf_rollups_for_range(pool, start, end).await?;
    let mut by_stage = BTreeMap::new();
    for ((bucket_start_epoch, stage), archive_delta) in archive_perf_by_bucket_stage {
        let materialized_overlap = build_materialized_pending_proxy_perf_overlap(
            materialized_by_bucket_stage.get(&(bucket_start_epoch, stage.clone())),
            materialized_bucket_sources
                .contains(&(bucket_start_epoch, SOURCE_PROXY.to_string()))
                .then(|| {
                    materialized_archive_perf_by_bucket_stage
                        .get(&(bucket_start_epoch, stage.clone()))
                })
                .flatten(),
        );
        let Some(delta) =
            build_proxy_perf_stage_rollup_delta(&archive_delta, materialized_overlap.as_ref())?
        else {
            continue;
        };
        let entry = by_stage
            .entry(stage)
            .or_insert_with(ProxyPerfStageHourlyDelta::default);
        entry.sample_count += delta.sample_count;
        entry.sum_ms += delta.sum_ms;
        entry.max_ms = entry.max_ms.max(delta.max_ms);
        if entry.histogram.is_empty() {
            entry.histogram = empty_approx_histogram();
        }
        merge_approx_histogram_into(&mut entry.histogram, &delta.histogram)?;
    }

    Ok(by_stage)
}

pub(crate) async fn query_unmaterialized_invocation_archive_totals(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
) -> Result<StatsTotals> {
    let mut totals = StatsTotals::default();
    for row in
        query_unmaterialized_invocation_archive_hourly_rollup_deltas(pool, source_scope, range)
            .await?
    {
        totals.total_count += row.total_count;
        totals.success_count += row.success_count;
        totals.failure_count += row.failure_count;
        totals.total_tokens += row.total_tokens;
        totals.total_cost += row.total_cost;
    }

    Ok(totals)
}

async fn load_failure_rows_from_archive_pool(
    archive_pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
) -> Result<Vec<ArchivedInvocationFailureRow>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT occurred_at, source, status, error_message, failure_kind, failure_class, is_actionable FROM codex_invocations WHERE occurred_at >= ",
    );
    query.push_bind(db_occurred_at_lower_bound(start));
    query
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_lower_bound(end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query
        .build_query_as::<ArchivedInvocationFailureRow>()
        .fetch_all(archive_pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn load_unmaterialized_invocation_archive_failure_rows(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
) -> Result<Vec<ArchivedInvocationFailureRow>> {
    let archive_rows = load_invocation_archives_missing_rollup_target(
        pool,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
    )
    .await?;
    let mut archive_failure_rows = Vec::new();

    for archive_row in archive_rows {
        let Some((archive_pool, temp_cleanup)) =
            open_invocation_archive_batch_pool(&archive_row, "failure-breakdown").await?
        else {
            continue;
        };
        let batch_rows =
            load_failure_rows_from_archive_pool(&archive_pool, start, end, source_scope).await?;
        archive_failure_rows.extend(batch_rows);
        archive_pool.close().await;
        drop(temp_cleanup);
    }

    let materialized_bucket_sources = load_materialized_rollup_bucket_sources(
        pool,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        &archive_failure_rows
            .iter()
            .filter_map(|row| {
                summary_rollup_bucket_start_epoch(&row.occurred_at)
                    .ok()
                    .map(|bucket_start_epoch| (bucket_start_epoch, row.source.clone()))
            })
            .collect::<HashSet<_>>(),
    )
    .await?;
    let relevant_failure_keys = archive_failure_rows
        .iter()
        .filter_map(|row| archived_failure_rollup_key(row).ok().flatten())
        .filter(|(bucket_start_epoch, source, _, _, _)| {
            materialized_bucket_sources.contains(&(*bucket_start_epoch, source.clone()))
        })
        .collect::<HashSet<_>>();
    let completed_archive_row_counts = load_materialized_failure_rollup_row_counts_for_keys(
        pool,
        start,
        end,
        source_scope,
        &relevant_failure_keys,
    )
    .await?;
    let missing_row_counts = load_missing_failure_rollup_row_counts_for_rows(
        pool,
        &archive_failure_rows,
        &materialized_bucket_sources,
        &completed_archive_row_counts,
    )
    .await?;
    if missing_row_counts.is_empty() {
        return Ok(Vec::new());
    }

    let mut emitted_row_counts = HashMap::<(i64, String, String, i64, String), usize>::new();
    Ok(archive_failure_rows
        .into_iter()
        .filter(|row| {
            let Ok(Some(key)) = archived_failure_rollup_key(row) else {
                return false;
            };
            let Some(missing_count) = missing_row_counts.get(&key) else {
                return false;
            };
            let emitted_count = emitted_row_counts.entry(key).or_default();
            if *emitted_count >= *missing_count {
                return false;
            }
            *emitted_count += 1;
            true
        })
        .collect())
}

async fn load_materialized_failure_rollup_row_counts_for_keys(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    keys: &HashSet<(i64, String, String, i64, String)>,
) -> Result<HashMap<(i64, String, String, i64, String), usize>> {
    if keys.is_empty() {
        return Ok(HashMap::new());
    }

    let month_keys = shanghai_month_keys_for_bucket_starts(
        keys.iter()
            .map(|(bucket_start_epoch, _, _, _, _)| *bucket_start_epoch),
    );
    let archive_paths = load_replayed_invocation_archive_paths_for_month_keys(
        pool,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        &month_keys,
    )
    .await?;
    let mut counts = HashMap::<(i64, String, String, i64, String), usize>::new();

    for archive_path in archive_paths {
        let archive_row = materialized_archive_path_row(archive_path);
        let Some((archive_pool, temp_cleanup)) =
            open_invocation_archive_batch_pool(&archive_row, "failure-breakdown").await?
        else {
            continue;
        };
        let batch_rows =
            load_failure_rows_from_archive_pool(&archive_pool, start, end, source_scope).await?;
        for row in batch_rows {
            let Some(key) = archived_failure_rollup_key(&row)? else {
                continue;
            };
            if !keys.contains(&key) {
                continue;
            }
            *counts.entry(key).or_default() += 1;
        }
        archive_pool.close().await;
        drop(temp_cleanup);
    }

    Ok(counts)
}

async fn rebuild_invocation_summary_rollups_from_archive_batch(
    tx: &mut SqliteConnection,
    archive_row: &ArchiveBatchPathRow,
    source_scope: InvocationSourceScope,
    seen_ids: &mut HashSet<i64>,
    cleared_rollup_buckets: &mut ClearedSummaryRollupBuckets,
    targets: &[&str],
    replace_existing_rollups: bool,
) -> Result<()> {
    if targets.is_empty() {
        return Ok(());
    }

    let archive_path = PathBuf::from(&archive_row.file_path);
    if !archive_path.exists() {
        bail!(
            "{}: {}",
            MISSING_INVOCATION_ARCHIVE_REPAIR_PREFIX,
            archive_row.file_path
        );
    }

    let mut cursor_id = 0_i64;
    let temp_path = PathBuf::from(format!(
        "{}.{}.sqlite",
        archive_path.display(),
        retention_temp_suffix()
    ));
    if temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }
    let temp_cleanup = TempSqliteCleanup(temp_path.clone());
    inflate_gzip_sqlite_file(&archive_path, &temp_path)?;
    let archive_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_for_path(&temp_path))
        .await
        .with_context(|| format!("failed to open archive batch {}", archive_path.display()))?;

    loop {
        let mut rows = load_invocation_hourly_source_rows_after_id(
            &archive_pool,
            cursor_id,
            source_scope,
            BACKFILL_BATCH_SIZE,
        )
        .await?;
        if rows.is_empty() {
            break;
        }
        cursor_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
        rows.retain(|row| seen_ids.insert(row.id));
        if rows.is_empty() {
            continue;
        }
        if replace_existing_rollups {
            for row in &rows {
                let bucket_start_epoch = summary_rollup_bucket_start_epoch(&row.occurred_at)?;
                let key = (bucket_start_epoch, row.source.clone());
                let targets_to_clear =
                    cleared_rollup_buckets.targets_to_clear_for_bucket(&key, targets);
                if targets_to_clear.is_empty() {
                    continue;
                }
                delete_invocation_summary_rollup_bucket_tx(tx, key.0, &key.1, &targets_to_clear)
                    .await?;
            }
        }
        upsert_invocation_hourly_rollups_tx(tx, &rows, targets).await?;
    }

    archive_pool.close().await;
    drop(temp_cleanup);

    for target in targets {
        mark_hourly_rollup_archive_replayed_tx(
            tx,
            target,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_row.file_path,
        )
        .await?;
    }

    Ok(())
}

fn summary_rollup_bucket_start_epoch(occurred_at: &str) -> Result<i64> {
    let occurred_at_utc = parse_to_utc_datetime(occurred_at)
        .ok_or_else(|| anyhow!("failed to parse invocation occurred_at: {occurred_at}"))?;
    Ok(align_bucket_epoch(occurred_at_utc.timestamp(), 3600, 0))
}

async fn delete_invocation_summary_rollup_bucket_tx(
    tx: &mut SqliteConnection,
    bucket_start_epoch: i64,
    source: &str,
    targets: &[&str],
) -> Result<()> {
    if targets.contains(&HOURLY_ROLLUP_TARGET_INVOCATIONS) {
        sqlx::query(
            "DELETE FROM invocation_rollup_hourly WHERE bucket_start_epoch = ?1 AND source = ?2",
        )
        .bind(bucket_start_epoch)
        .bind(source)
        .execute(&mut *tx)
        .await?;
    }
    if targets.contains(&HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES) {
        sqlx::query(
            "DELETE FROM invocation_failure_rollup_hourly WHERE bucket_start_epoch = ?1 AND source = ?2",
        )
        .bind(bucket_start_epoch)
        .bind(source)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

async fn load_live_invocation_summary_rows_for_cleared_buckets_up_to_id(
    tx: &mut SqliteConnection,
    cleared_rollup_buckets: &HashSet<(i64, String)>,
    source_scope: InvocationSourceScope,
    end_at_id: i64,
) -> Result<Vec<InvocationHourlySourceRecord>> {
    if cleared_rollup_buckets.is_empty() || end_at_id <= 0 {
        return Ok(Vec::new());
    }

    let min_bucket_epoch = cleared_rollup_buckets
        .iter()
        .map(|(bucket_start_epoch, _)| *bucket_start_epoch)
        .min()
        .ok_or_else(|| anyhow!("missing minimum cleared summary rollup bucket epoch"))?;
    let max_bucket_epoch = cleared_rollup_buckets
        .iter()
        .map(|(bucket_start_epoch, _)| *bucket_start_epoch)
        .max()
        .ok_or_else(|| anyhow!("missing maximum cleared summary rollup bucket epoch"))?;
    let min_bucket_start = Utc
        .timestamp_opt(min_bucket_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid minimum cleared summary rollup bucket epoch"))?;
    let max_bucket_end = Utc
        .timestamp_opt(max_bucket_epoch + 3_600, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid maximum cleared summary rollup bucket epoch"))?;

    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            id,
            occurred_at,
            source,
            status,
            detail_level,
            total_tokens,
            cost,
            error_message,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
        FROM codex_invocations
        WHERE id <=
        "#,
    );
    query
        .push_bind(end_at_id)
        .push(" AND occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(min_bucket_start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_lower_bound(max_bucket_end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" ORDER BY id ASC");

    let rows = query
        .build_query_as::<InvocationHourlySourceRecord>()
        .fetch_all(&mut *tx)
        .await?;
    Ok(rows
        .into_iter()
        .filter(|row| {
            summary_rollup_bucket_start_epoch(&row.occurred_at)
                .map(|bucket_start_epoch| {
                    cleared_rollup_buckets.contains(&(bucket_start_epoch, row.source.clone()))
                })
                .unwrap_or(false)
        })
        .collect())
}

async fn rebuild_invocation_summary_rollups_from_live_rows(
    tx: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    seen_ids: &mut HashSet<i64>,
    targets: &[&str],
    start_after_id: i64,
) -> Result<i64> {
    let mut cursor_id = start_after_id;
    loop {
        let mut rows = load_invocation_hourly_source_rows_after_id(
            &mut *tx,
            cursor_id,
            source_scope,
            BACKFILL_BATCH_SIZE,
        )
        .await?;
        if rows.is_empty() {
            break;
        }
        cursor_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
        rows.retain(|row| seen_ids.insert(row.id));
        if rows.is_empty() {
            continue;
        }
        upsert_invocation_hourly_rollups_tx(tx, &rows, targets).await?;
    }
    Ok(cursor_id)
}

async fn mark_materialized_invocation_summary_archive_replayed_tx(
    tx: &mut SqliteConnection,
    archive_row: &ArchiveBatchPathRow,
) -> Result<()> {
    for target in INVOCATION_SUMMARY_ROLLUP_TARGETS {
        mark_hourly_rollup_archive_replayed_tx(
            tx,
            target,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_row.file_path,
        )
        .await?;
    }
    Ok(())
}

async fn hourly_rollup_progress_exists(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    dataset: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT 1 FROM hourly_rollup_live_progress WHERE dataset = ?1 LIMIT 1",
    )
    .bind(dataset)
    .fetch_optional(executor)
    .await?
    .is_some())
}

async fn repair_invocation_summary_rollups(pool: &Pool<Sqlite>) -> Result<()> {
    if load_hourly_rollup_live_progress(pool, INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DATASET)
        .await?
        >= INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DONE
        && hourly_rollup_progress_exists(
            pool,
            INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET,
        )
        .await?
    {
        return Ok(());
    }

    let mut tx = pool.begin().await?;
    if load_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DATASET,
    )
    .await?
        >= INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DONE
        && hourly_rollup_progress_exists(
            tx.as_mut(),
            INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET,
        )
        .await?
    {
        tx.rollback().await?;
        return Ok(());
    }

    let archive_rows = load_completed_invocation_archive_paths(tx.as_mut()).await?;
    let preserve_materialized_archives = archive_rows.iter().any(|archive_row| {
        archive_row.historical_rollups_materialized_at.is_some()
            && !PathBuf::from(&archive_row.file_path).exists()
    });
    let shared_live_cursor =
        load_hourly_rollup_live_progress_tx(tx.as_mut(), HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;

    if !preserve_materialized_archives {
        sqlx::query("DELETE FROM invocation_rollup_hourly")
            .execute(tx.as_mut())
            .await?;
        sqlx::query("DELETE FROM invocation_failure_rollup_hourly")
            .execute(tx.as_mut())
            .await?;
    }

    let mut seen_ids = HashSet::new();
    let mut cleared_rollup_buckets = ClearedSummaryRollupBuckets::default();
    for archive_row in &archive_rows {
        let preserve_materialized_archive =
            archive_row.historical_rollups_materialized_at.is_some()
                && !PathBuf::from(&archive_row.file_path).exists();
        if preserve_materialized_archive {
            mark_materialized_invocation_summary_archive_replayed_tx(tx.as_mut(), archive_row)
                .await?;
            continue;
        }
        rebuild_invocation_summary_rollups_from_archive_batch(
            tx.as_mut(),
            archive_row,
            InvocationSourceScope::All,
            &mut seen_ids,
            &mut cleared_rollup_buckets,
            &INVOCATION_SUMMARY_ROLLUP_TARGETS,
            preserve_materialized_archives,
        )
        .await?;
    }
    let mut restored_live_rows = load_live_invocation_summary_rows_for_cleared_buckets_up_to_id(
        tx.as_mut(),
        &cleared_rollup_buckets.overall,
        InvocationSourceScope::All,
        shared_live_cursor,
    )
    .await?;
    restored_live_rows.retain(|row| !seen_ids.contains(&row.id));
    if !restored_live_rows.is_empty() {
        upsert_invocation_hourly_rollups_tx(
            tx.as_mut(),
            &restored_live_rows,
            &INVOCATION_SUMMARY_ROLLUP_TARGETS,
        )
        .await?;
    }
    let live_cursor_id = rebuild_invocation_summary_rollups_from_live_rows(
        tx.as_mut(),
        InvocationSourceScope::All,
        &mut seen_ids,
        &INVOCATION_SUMMARY_ROLLUP_TARGETS,
        if preserve_materialized_archives {
            shared_live_cursor
        } else {
            0
        },
    )
    .await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET,
        live_cursor_id.max(shared_live_cursor),
    )
    .await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DATASET,
        INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DONE,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn backfill_missing_invocation_summary_archive_rollups(pool: &Pool<Sqlite>) -> Result<()> {
    let archive_rows = load_invocation_archives_missing_summary_rollup_markers(pool).await?;
    if archive_rows.is_empty() {
        return Ok(());
    }

    let mut tx = pool.begin().await?;
    let archive_rows = load_invocation_archives_missing_summary_rollup_markers(tx.as_mut()).await?;
    if archive_rows.is_empty() {
        tx.rollback().await?;
        return Ok(());
    }

    let shared_live_cursor =
        load_hourly_rollup_live_progress_tx(tx.as_mut(), HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let mut seen_ids = HashSet::new();
    let mut cleared_rollup_buckets = ClearedSummaryRollupBuckets::default();
    for archive_row in &archive_rows {
        let needs_overall = archive_row.needs_overall.unwrap_or_default() != 0;
        let needs_failures = archive_row.needs_failures.unwrap_or_default() != 0;
        let mut targets = Vec::new();
        if needs_overall {
            targets.push(HOURLY_ROLLUP_TARGET_INVOCATIONS);
        }
        if needs_failures {
            targets.push(HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES);
        }
        if targets.is_empty() {
            continue;
        }
        let archive_path = PathBuf::from(&archive_row.file_path);
        if archive_row.historical_rollups_materialized_at.is_some() && !archive_path.exists() {
            for target in &targets {
                mark_hourly_rollup_archive_replayed_tx(
                    tx.as_mut(),
                    target,
                    HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    &archive_row.file_path,
                )
                .await?;
            }
            continue;
        }
        rebuild_invocation_summary_rollups_from_archive_batch(
            tx.as_mut(),
            archive_row,
            InvocationSourceScope::All,
            &mut seen_ids,
            &mut cleared_rollup_buckets,
            &targets,
            true,
        )
        .await?;
    }
    let mut restored_overall_live_rows =
        load_live_invocation_summary_rows_for_cleared_buckets_up_to_id(
            tx.as_mut(),
            &cleared_rollup_buckets.overall,
            InvocationSourceScope::All,
            shared_live_cursor,
        )
        .await?;
    restored_overall_live_rows.retain(|row| !seen_ids.contains(&row.id));
    if !restored_overall_live_rows.is_empty() {
        upsert_invocation_hourly_rollups_tx(
            tx.as_mut(),
            &restored_overall_live_rows,
            &[HOURLY_ROLLUP_TARGET_INVOCATIONS],
        )
        .await?;
    }
    let mut restored_failure_live_rows =
        load_live_invocation_summary_rows_for_cleared_buckets_up_to_id(
            tx.as_mut(),
            &cleared_rollup_buckets.failures,
            InvocationSourceScope::All,
            shared_live_cursor,
        )
        .await?;
    restored_failure_live_rows.retain(|row| !seen_ids.contains(&row.id));
    if !restored_failure_live_rows.is_empty() {
        upsert_invocation_hourly_rollups_tx(
            tx.as_mut(),
            &restored_failure_live_rows,
            &[HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn ensure_invocation_summary_rollups_ready(pool: &Pool<Sqlite>) -> Result<()> {
    if load_completed_invocation_archive_paths(pool)
        .await?
        .is_empty()
    {
        return Ok(());
    }

    repair_invocation_summary_rollups(pool).await?;
    backfill_missing_invocation_summary_archive_rollups(pool).await?;
    Ok(())
}

pub(crate) async fn ensure_invocation_summary_rollups_ready_best_effort(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    match ensure_invocation_summary_rollups_ready(pool).await {
        Ok(()) => Ok(()),
        Err(err) if is_missing_invocation_summary_archive_error(&err) => {
            warn!(
                error = %err,
                "skipping invocation summary rollup repair because an archive batch file is missing; reusing current rollups for historical range queries"
            );
            Ok(())
        }
        Err(err) => Err(err),
    }
}

async fn query_invocation_all_time_rollup_totals(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            COALESCE(SUM(total_count), 0) AS total_count,
            COALESCE(SUM(success_count), 0) AS success_count,
            COALESCE(SUM(failure_count), 0) AS failure_count,
            COALESCE(SUM(total_cost), 0.0) AS total_cost,
            COALESCE(SUM(total_tokens), 0) AS total_tokens
        FROM invocation_rollup_hourly
        WHERE 1 = 1
        "#,
    );
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    let mut totals = StatsTotals::from(query.build_query_as::<StatsRow>().fetch_one(pool).await?);
    let live_progress_cursor =
        load_hourly_rollup_live_progress(pool, HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let repair_live_cursor = load_hourly_rollup_live_progress(
        pool,
        INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET,
    )
    .await?;
    let tail_cursor = live_progress_cursor.max(repair_live_cursor);
    if tail_cursor <= 0 {
        return Ok(totals);
    }

    let tail_query = match source_scope {
        InvocationSourceScope::ProxyOnly => format!(
            "SELECT {} FROM codex_invocations WHERE id > ?1 AND source = ?2",
            stats_success_failure_select_sql()
        ),
        InvocationSourceScope::All => format!(
            "SELECT {} FROM codex_invocations WHERE id > ?1",
            stats_success_failure_select_sql()
        ),
    };
    let tail = match source_scope {
        InvocationSourceScope::ProxyOnly => {
            sqlx::query_as::<_, StatsRow>(&tail_query)
                .bind(tail_cursor)
                .bind(SOURCE_PROXY)
                .fetch_one(pool)
                .await?
        }
        InvocationSourceScope::All => {
            sqlx::query_as::<_, StatsRow>(&tail_query)
                .bind(tail_cursor)
                .fetch_one(pool)
                .await?
        }
    };
    totals = totals.add(StatsTotals::from(tail));
    Ok(totals)
}

pub(crate) async fn query_invocation_totals(
    pool: &Pool<Sqlite>,
    filter: StatsFilter,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals> {
    if matches!(filter, StatsFilter::All) {
        if load_completed_invocation_archive_paths(pool)
            .await?
            .is_empty()
        {
            return Ok(StatsTotals::from(
                query_stats_row(pool, StatsFilter::All, source_scope).await?,
            ));
        }

        // Read paths must stay query-only even when historical summary repair is still pending.
        // Background startup / follow-up maintenance is responsible for rebuilding stale archived
        // hourly rollups and summary replay markers; requests reuse the current materialized
        // rollups plus any still-unmaterialized archive batches instead of writing through here.
        return Ok(query_invocation_all_time_rollup_totals(pool, source_scope)
            .await?
            .add(query_unmaterialized_invocation_archive_totals(pool, source_scope, None).await?));
    }

    Ok(StatsTotals::from(
        query_stats_row(pool, filter, source_scope).await?,
    ))
}

pub(crate) async fn query_invocation_hourly_rollup_range(
    pool: &Pool<Sqlite>,
    range_start_epoch: i64,
    range_end_epoch: i64,
    source_scope: InvocationSourceScope,
) -> Result<Vec<InvocationHourlyRollupRecord>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            bucket_start_epoch,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram,
            first_response_byte_total_sample_count,
            first_response_byte_total_sum_ms,
            first_response_byte_total_max_ms,
            first_response_byte_total_histogram
        FROM invocation_rollup_hourly
        WHERE bucket_start_epoch >=
        "#,
    );
    query.push_bind(range_start_epoch);
    query
        .push(" AND bucket_start_epoch < ")
        .push_bind(range_end_epoch);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" ORDER BY bucket_start_epoch ASC");

    query
        .build_query_as::<InvocationHourlyRollupRecord>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_invocation_failure_hourly_rollup_range(
    pool: &Pool<Sqlite>,
    range_start_epoch: i64,
    range_end_epoch: i64,
    source_scope: InvocationSourceScope,
) -> Result<Vec<InvocationFailureHourlyRollupRecord>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            failure_class,
            is_actionable,
            error_category,
            SUM(failure_count) AS failure_count
        FROM invocation_failure_rollup_hourly
        WHERE bucket_start_epoch >=
        "#,
    );
    query.push_bind(range_start_epoch);
    query
        .push(" AND bucket_start_epoch < ")
        .push_bind(range_end_epoch);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" GROUP BY failure_class, is_actionable, error_category");

    query
        .build_query_as::<InvocationFailureHourlyRollupRecord>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_proxy_perf_stage_hourly_rollup_range(
    pool: &Pool<Sqlite>,
    range_start_epoch: i64,
    range_end_epoch: i64,
) -> Result<Vec<ProxyPerfStageHourlyRollupRecord>> {
    sqlx::query_as::<_, ProxyPerfStageHourlyRollupRecord>(
        r#"
        SELECT
            bucket_start_epoch,
            stage,
            sample_count,
            sum_ms,
            max_ms,
            histogram
        FROM proxy_perf_stage_hourly
        WHERE bucket_start_epoch >= ?1
          AND bucket_start_epoch < ?2
        ORDER BY stage ASC, bucket_start_epoch ASC
        "#,
    )
    .bind(range_start_epoch)
    .bind(range_end_epoch)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub(crate) async fn query_crs_totals(
    pool: &Pool<Sqlite>,
    relay: Option<&CrsStatsConfig>,
    filter: &StatsFilter,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals> {
    if source_scope == InvocationSourceScope::ProxyOnly {
        return Ok(StatsTotals::default());
    }
    let relay = match relay {
        Some(relay) => relay,
        None => return Ok(StatsTotals::default()),
    };
    let mut query = String::from(
        r#"
        SELECT
            COALESCE(SUM(total_count), 0) AS total_count,
            COALESCE(SUM(success_count), 0) AS success_count,
            COALESCE(SUM(failure_count), 0) AS failure_count,
            COALESCE(SUM(total_cost), 0.0) AS total_cost,
            COALESCE(SUM(total_tokens), 0) AS total_tokens
        FROM stats_source_deltas
        WHERE source = ?1 AND period = ?2
        "#,
    );

    let mut binds: Vec<i64> = Vec::new();
    match filter {
        StatsFilter::Since(start) => {
            query.push_str(" AND captured_at_epoch >= ?3");
            binds.push(start.timestamp());
        }
        StatsFilter::Range(start, end) => {
            query.push_str(" AND captured_at_epoch >= ?3 AND captured_at_epoch < ?4");
            binds.push(start.timestamp());
            binds.push(exclusive_epoch_upper_bound(*end));
        }
        StatsFilter::RecentLimit(_) => {
            return Ok(StatsTotals::default());
        }
        StatsFilter::All => {}
    }

    let mut sql = sqlx::query_as::<_, StatsRow>(&query)
        .bind(SOURCE_CRS)
        .bind(&relay.period);

    for epoch in binds {
        sql = sql.bind(epoch);
    }

    let row = sql.fetch_one(pool).await?;
    Ok(StatsTotals::from(row))
}

pub(crate) async fn query_combined_totals(
    pool: &Pool<Sqlite>,
    relay: Option<&CrsStatsConfig>,
    filter: StatsFilter,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals> {
    let base = query_invocation_totals(pool, filter.clone(), source_scope).await?;
    let relay_totals = query_crs_totals(pool, relay, &filter, source_scope).await?;
    Ok(base.add(relay_totals))
}

pub(crate) async fn resolve_default_source_scope(
    _pool: &Pool<Sqlite>,
) -> Result<InvocationSourceScope> {
    Ok(InvocationSourceScope::All)
}

pub(crate) async fn query_crs_deltas(
    pool: &Pool<Sqlite>,
    relay: &CrsStatsConfig,
    start_epoch: i64,
    end_epoch_exclusive: i64,
) -> Result<Vec<StatsDeltaRecord>> {
    sqlx::query_as::<_, StatsDeltaRecord>(
        r#"
        SELECT
            captured_at_epoch,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost
        FROM stats_source_deltas
        WHERE source = ?1
          AND period = ?2
          AND captured_at_epoch >= ?3
          AND captured_at_epoch < ?4
        ORDER BY captured_at_epoch ASC
        "#,
    )
    .bind(SOURCE_CRS)
    .bind(&relay.period)
    .bind(start_epoch)
    .bind(end_epoch_exclusive)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CrsStatsResponse {
    pub(crate) success: bool,
    #[serde(default)]
    pub(crate) data: Vec<CrsModelStats>,
    #[serde(default)]
    pub(crate) period: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CrsModelStats {
    pub(crate) model: String,
    pub(crate) requests: i64,
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) cache_create_tokens: i64,
    pub(crate) cache_read_tokens: i64,
    pub(crate) all_tokens: i64,
    pub(crate) costs: CrsCosts,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CrsCosts {
    pub(crate) input: f64,
    pub(crate) output: f64,
    pub(crate) cache_write: f64,
    pub(crate) cache_read: f64,
    pub(crate) total: f64,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct CrsTotals {
    pub(crate) total_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) cache_create_tokens: i64,
    pub(crate) cache_read_tokens: i64,
    pub(crate) cost_input: f64,
    pub(crate) cost_output: f64,
    pub(crate) cost_cache_write: f64,
    pub(crate) cost_cache_read: f64,
}

#[derive(Debug)]
pub(crate) enum ApiError {
    BadRequest(anyhow::Error),
    Internal(anyhow::Error),
}

impl ApiError {
    pub(crate) fn bad_request<E>(err: E) -> Self
    where
        E: Into<anyhow::Error>,
    {
        Self::BadRequest(err.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, err) = match self {
            ApiError::BadRequest(err) => (StatusCode::BAD_REQUEST, err),
            ApiError::Internal(err) => (StatusCode::INTERNAL_SERVER_ERROR, err),
        };
        let message = format!("{err}");
        (status, message).into_response()
    }
}

impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self::Internal(err.into())
    }
}

// --- ISO8601 UTC helpers and serializers ---
pub(crate) fn format_utc_iso(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub(crate) fn format_utc_iso_millis(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(crate) fn format_utc_iso_precise(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

pub(crate) fn parse_to_utc_datetime(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        if let Some(loc) = Shanghai.from_local_datetime(&naive).single() {
            return Some(loc.with_timezone(&Utc));
        }
        return Some(Utc.from_utc_datetime(&naive));
    }
    None
}

#[allow(clippy::ptr_arg)]
pub(crate) fn serialize_local_naive_to_utc_iso<S>(
    value: &String,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let iso = parse_to_utc_datetime(value)
        .map(format_utc_iso)
        .unwrap_or_else(|| value.clone());
    serializer.serialize_str(&iso)
}

#[allow(clippy::ptr_arg)]
pub(crate) fn serialize_local_or_utc_to_utc_iso<S>(
    value: &String,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serialize_local_naive_to_utc_iso(value, serializer)
}

#[allow(clippy::ptr_arg)]
pub(crate) fn serialize_opt_local_or_utc_to_utc_iso<S>(
    value: &Option<String>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match value {
        Some(s) => serialize_local_naive_to_utc_iso(s, serializer),
        None => serializer.serialize_none(),
    }
}
