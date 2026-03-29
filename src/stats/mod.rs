use super::*;

#[derive(Debug)]
pub(crate) enum SummaryWindow {
    All,
    Current(i64),
    Duration(ChronoDuration),
    Calendar(String),
}

#[derive(Debug, Clone)]
pub(crate) enum StatsFilter {
    All,
    Since(DateTime<Utc>),
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
        if end == now && end.timestamp_subsec_nanos() == 0 {
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
        Some(raw @ ("today" | "thisWeek" | "thisMonth")) => {
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
        (StatsFilter::All, InvocationSourceScope::ProxyOnly) => sqlx::query_as::<_, StatsRow>(
            r#"
                SELECT
                    COUNT(*) AS total_count,
                    SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) AS success_count,
                    SUM(CASE WHEN status != 'success' THEN 1 ELSE 0 END) AS failure_count,
                    COALESCE(SUM(cost), 0.0) AS total_cost,
                    COALESCE(SUM(total_tokens), 0) AS total_tokens
                FROM codex_invocations
                WHERE source = ?1
                "#,
        )
        .bind(SOURCE_PROXY)
        .fetch_one(pool)
        .await
        .map_err(Into::into),
        (StatsFilter::All, InvocationSourceScope::All) => sqlx::query_as::<_, StatsRow>(
            r#"
                SELECT
                    COUNT(*) AS total_count,
                    SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) AS success_count,
                    SUM(CASE WHEN status != 'success' THEN 1 ELSE 0 END) AS failure_count,
                    COALESCE(SUM(cost), 0.0) AS total_cost,
                    COALESCE(SUM(total_tokens), 0) AS total_tokens
                FROM codex_invocations
                "#,
        )
        .fetch_one(pool)
        .await
        .map_err(Into::into),
        (StatsFilter::Since(start), InvocationSourceScope::ProxyOnly) => {
            sqlx::query_as::<_, StatsRow>(
                r#"
                SELECT
                    COUNT(*) AS total_count,
                    SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) AS success_count,
                    SUM(CASE WHEN status != 'success' THEN 1 ELSE 0 END) AS failure_count,
                    COALESCE(SUM(cost), 0.0) AS total_cost,
                    COALESCE(SUM(total_tokens), 0) AS total_tokens
                FROM codex_invocations
                WHERE source = ?1 AND occurred_at >= ?2
                "#,
            )
            .bind(SOURCE_PROXY)
            .bind(db_occurred_at_lower_bound(start))
            .fetch_one(pool)
            .await
            .map_err(Into::into)
        }
        (StatsFilter::Since(start), InvocationSourceScope::All) => sqlx::query_as::<_, StatsRow>(
            r#"
                SELECT
                    COUNT(*) AS total_count,
                    SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) AS success_count,
                    SUM(CASE WHEN status != 'success' THEN 1 ELSE 0 END) AS failure_count,
                    COALESCE(SUM(cost), 0.0) AS total_cost,
                    COALESCE(SUM(total_tokens), 0) AS total_tokens
                FROM codex_invocations
                WHERE occurred_at >= ?1
                "#,
        )
        .bind(db_occurred_at_lower_bound(start))
        .fetch_one(pool)
        .await
        .map_err(Into::into),
        (StatsFilter::RecentLimit(limit), InvocationSourceScope::ProxyOnly) => {
            sqlx::query_as::<_, StatsRow>(
                r#"
                WITH recent AS (
                    SELECT *
                    FROM codex_invocations
                    WHERE source = ?1
                    ORDER BY occurred_at DESC
                    LIMIT ?2
                )
                SELECT
                    COUNT(*) AS total_count,
                    SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) AS success_count,
                    SUM(CASE WHEN status != 'success' THEN 1 ELSE 0 END) AS failure_count,
                    COALESCE(SUM(cost), 0.0) AS total_cost,
                    COALESCE(SUM(total_tokens), 0) AS total_tokens
                FROM recent
                "#,
            )
            .bind(SOURCE_PROXY)
            .bind(limit)
            .fetch_one(pool)
            .await
            .map_err(Into::into)
        }
        (StatsFilter::RecentLimit(limit), InvocationSourceScope::All) => {
            sqlx::query_as::<_, StatsRow>(
                r#"
                WITH recent AS (
                    SELECT *
                    FROM codex_invocations
                    ORDER BY occurred_at DESC
                    LIMIT ?1
                )
                SELECT
                    COUNT(*) AS total_count,
                    SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) AS success_count,
                    SUM(CASE WHEN status != 'success' THEN 1 ELSE 0 END) AS failure_count,
                    COALESCE(SUM(cost), 0.0) AS total_cost,
                    COALESCE(SUM(total_tokens), 0) AS total_tokens
                FROM recent
                "#,
            )
            .bind(limit)
            .fetch_one(pool)
            .await
            .map_err(Into::into)
        }
    }
}

pub(crate) async fn query_invocation_totals(
    pool: &Pool<Sqlite>,
    filter: StatsFilter,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals> {
    if matches!(filter, StatsFilter::All) {
        let mut totals = query_invocation_hourly_rollup_totals(pool, source_scope).await?;
        let last_row_id = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT cursor_id
            FROM hourly_rollup_live_progress
            WHERE dataset = 'codex_invocations'
            LIMIT 1
            "#,
        )
        .fetch_optional(pool)
        .await?
        .unwrap_or_default();
        let tail = match source_scope {
            InvocationSourceScope::ProxyOnly => {
                sqlx::query_as::<_, StatsRow>(
                    r#"
                SELECT
                    COUNT(*) AS total_count,
                    SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) AS success_count,
                    SUM(CASE WHEN status != 'success' THEN 1 ELSE 0 END) AS failure_count,
                    COALESCE(SUM(cost), 0.0) AS total_cost,
                    COALESCE(SUM(total_tokens), 0) AS total_tokens
                FROM codex_invocations
                WHERE id > ?1 AND source = ?2
                "#,
                )
                .bind(last_row_id)
                .bind(SOURCE_PROXY)
                .fetch_one(pool)
                .await?
            }
            InvocationSourceScope::All => {
                sqlx::query_as::<_, StatsRow>(
                    r#"
                SELECT
                    COUNT(*) AS total_count,
                    SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) AS success_count,
                    SUM(CASE WHEN status != 'success' THEN 1 ELSE 0 END) AS failure_count,
                    COALESCE(SUM(cost), 0.0) AS total_cost,
                    COALESCE(SUM(total_tokens), 0) AS total_tokens
                FROM codex_invocations
                WHERE id > ?1
                "#,
                )
                .bind(last_row_id)
                .fetch_one(pool)
                .await?
            }
        };
        totals = totals.add(StatsTotals::from(tail));
        return Ok(totals);
    }

    Ok(StatsTotals::from(
        query_stats_row(pool, filter, source_scope).await?,
    ))
}

pub(crate) async fn query_invocation_hourly_rollup_totals(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals> {
    let row = match source_scope {
        InvocationSourceScope::ProxyOnly => {
            sqlx::query_as::<_, StatsRow>(
                r#"
                SELECT
                    COALESCE(SUM(total_count), 0) AS total_count,
                    COALESCE(SUM(success_count), 0) AS success_count,
                    COALESCE(SUM(failure_count), 0) AS failure_count,
                    COALESCE(SUM(total_cost), 0.0) AS total_cost,
                    COALESCE(SUM(total_tokens), 0) AS total_tokens
                FROM invocation_rollup_hourly
                WHERE source = ?1
                "#,
            )
            .bind(SOURCE_PROXY)
            .fetch_one(pool)
            .await?
        }
        InvocationSourceScope::All => {
            sqlx::query_as::<_, StatsRow>(
                r#"
                SELECT
                    COALESCE(SUM(total_count), 0) AS total_count,
                    COALESCE(SUM(success_count), 0) AS success_count,
                    COALESCE(SUM(failure_count), 0) AS failure_count,
                    COALESCE(SUM(total_cost), 0.0) AS total_cost,
                    COALESCE(SUM(total_tokens), 0) AS total_tokens
                FROM invocation_rollup_hourly
                "#,
            )
            .fetch_one(pool)
            .await?
        }
    };
    Ok(StatsTotals::from(row))
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
    if let StatsFilter::Since(start) = filter {
        query.push_str(" AND captured_at_epoch >= ?3");
        binds.push(start.timestamp());
    } else if matches!(filter, StatsFilter::RecentLimit(_)) {
        return Ok(StatsTotals::default());
    }

    let mut sql = sqlx::query_as::<_, StatsRow>(&query)
        .bind(SOURCE_CRS)
        .bind(&relay.period);

    if let Some(epoch) = binds.first() {
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
    end_epoch: i64,
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
          AND captured_at_epoch <= ?4
        ORDER BY captured_at_epoch ASC
        "#,
    )
    .bind(SOURCE_CRS)
    .bind(&relay.period)
    .bind(start_epoch)
    .bind(end_epoch)
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
