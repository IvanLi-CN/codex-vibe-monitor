use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LongTermRange {
    Seven,
    Thirty,
    OneEighty,
    ThreeSixtyFive,
}

impl LongTermRange {
    pub(super) fn parse(raw: Option<&str>) -> Option<Self> {
        match raw.unwrap_or("7d") {
            "7d" => Some(Self::Seven),
            "30d" => Some(Self::Thirty),
            "180d" => Some(Self::OneEighty),
            "365d" => Some(Self::ThreeSixtyFive),
            _ => None,
        }
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Seven => "7d",
            Self::Thirty => "30d",
            Self::OneEighty => "180d",
            Self::ThreeSixtyFive => "365d",
        }
    }

    pub(super) fn days(self) -> i64 {
        match self {
            Self::Seven => 7,
            Self::Thirty => 30,
            Self::OneEighty => 180,
            Self::ThreeSixtyFive => 365,
        }
    }
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct LongTermRangeQuery {
    pub(crate) range: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct LongTermSeriesQuery {
    pub(crate) range: Option<String>,
    pub(crate) dimension: Option<String>,
    #[serde(default)]
    pub(crate) key: Vec<String>,
}

pub(super) fn parse_long_term_series_query(uri: &Uri) -> LongTermSeriesQuery {
    let mut query = LongTermSeriesQuery::default();
    for (key, value) in url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes()) {
        match key.as_ref() {
            "range" => query.range = Some(value.into_owned()),
            "dimension" => query.dimension = Some(value.into_owned()),
            "key" => query.key.push(value.into_owned()),
            _ => {}
        }
    }
    query
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LongTermMetrics {
    pub(crate) calls: i64,
    pub(crate) tokens: Option<i64>,
    pub(crate) token_samples: i64,
    pub(crate) cost: Option<f64>,
    pub(crate) cost_samples: i64,
    pub(crate) usage_time_ms: Option<f64>,
    pub(crate) usage_time_samples: i64,
    pub(crate) wall_time_ms: Option<f64>,
    pub(crate) wall_time_samples: i64,
    pub(crate) output_speed_tokens_per_second: Option<f64>,
    pub(crate) output_speed_samples: i64,
    pub(crate) first_byte_ms: Option<f64>,
    pub(crate) first_byte_samples: i64,
    pub(crate) response_ms: Option<f64>,
    pub(crate) response_samples: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LongTermDailyPoint {
    pub(crate) date: String,
    #[serde(flatten)]
    pub(crate) metrics: LongTermMetrics,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LongTermSeriesSummary {
    pub(crate) series_key: String,
    pub(crate) display_name: String,
    pub(crate) reasoning_effort: Option<String>,
    #[serde(flatten)]
    pub(crate) metrics: LongTermMetrics,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LongTermSeries {
    pub(crate) series_key: String,
    pub(crate) display_name: String,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) points: Vec<LongTermDailyPoint>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LongTermStatsOverviewResponse {
    pub(crate) status: String,
    pub(crate) statistics_start_date: Option<String>,
    pub(crate) processed_rows: i64,
    pub(crate) total_rows: i64,
    pub(crate) timezone: &'static str,
    pub(crate) range: String,
    pub(crate) global: LongTermMetrics,
    pub(crate) daily: Vec<LongTermDailyPoint>,
    pub(crate) models: Vec<LongTermSeriesSummary>,
    pub(crate) upstreams: Vec<LongTermSeriesSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LongTermStatsSeriesResponse {
    pub(crate) status: String,
    pub(crate) statistics_start_date: Option<String>,
    pub(crate) processed_rows: i64,
    pub(crate) total_rows: i64,
    pub(crate) timezone: &'static str,
    pub(crate) range: String,
    pub(crate) dimension: String,
    pub(crate) series: Vec<LongTermSeries>,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct LongTermStateRow {
    pub(super) status: String,
    pub(super) statistics_start_date: Option<String>,
    pub(super) integrity_source_start_date: Option<String>,
    pub(super) processed_rows: i64,
    pub(super) total_rows: i64,
    pub(super) last_error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LongTermProjectionHealth {
    pub(crate) state: String,
    pub(crate) cursor_row_id: i64,
    pub(crate) dirty_bucket_count: usize,
    pub(crate) pending_event_count: usize,
    pub(crate) last_flush_elapsed_ms: Option<u64>,
    pub(crate) last_flush_age_ms: Option<u64>,
    pub(crate) last_repair_scope: Option<String>,
    pub(crate) last_defer_reason: Option<String>,
    pub(crate) last_error_kind: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct LongTermProjectionRuntime {
    pub(crate) state: String,
    pub(crate) cursor_row_id: i64,
    pub(crate) dirty_bucket_count: usize,
    pub(crate) pending_event_count: usize,
    pub(crate) last_flush_elapsed_ms: Option<u64>,
    pub(crate) last_flush_at: Option<Instant>,
    pub(crate) last_repair_scope: Option<String>,
    pub(crate) last_defer_reason: Option<String>,
    pub(crate) last_error_kind: Option<String>,
    pub(super) next_repair_at: Option<Instant>,
    pub(super) interval_index:
        HashMap<LongTermProjectionIntervalKey, LongTermProjectionIntervalUnion>,
    pub(super) loaded_interval_dates: HashSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct LongTermProjectionIntervalKey {
    pub(super) bucket_kind: &'static str,
    pub(super) bucket_key: String,
    pub(super) dimension: String,
    pub(super) series_key: String,
}

#[derive(Debug, Clone, Default)]
pub(super) struct LongTermProjectionIntervalUnion {
    pub(super) intervals: BTreeMap<i64, i64>,
    pub(super) duration_ms: i64,
    pub(super) sample_count: i64,
}

impl LongTermProjectionIntervalUnion {
    pub(super) fn add(&mut self, mut start_ms: i64, mut end_ms: i64) {
        if end_ms <= start_ms {
            return;
        }
        if let Some((&previous_start, &previous_end)) =
            self.intervals.range(..=start_ms).next_back()
            && previous_end >= start_ms
        {
            start_ms = previous_start;
            end_ms = end_ms.max(previous_end);
            self.duration_ms = self
                .duration_ms
                .saturating_sub(previous_end.saturating_sub(previous_start));
            self.intervals.remove(&previous_start);
        }
        loop {
            let next = self
                .intervals
                .range(start_ms..)
                .next()
                .map(|(&next_start, &next_end)| (next_start, next_end));
            let Some((next_start, next_end)) = next else {
                break;
            };
            if next_start > end_ms {
                break;
            }
            end_ms = end_ms.max(next_end);
            self.duration_ms = self
                .duration_ms
                .saturating_sub(next_end.saturating_sub(next_start));
            self.intervals.remove(&next_start);
        }
        self.duration_ms = self
            .duration_ms
            .saturating_add(end_ms.saturating_sub(start_ms));
        self.intervals.insert(start_ms, end_ms);
        self.sample_count = self.sample_count.saturating_add(1);
    }
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct LongTermProjectionLegacyIntervalRow {
    pub(super) invocation_row_id: i64,
    pub(super) bucket_kind: String,
    pub(super) bucket_date: String,
    pub(super) bucket_key: String,
    pub(super) dimension: String,
    pub(super) series_key: String,
    pub(super) interval_start_ms: i64,
    pub(super) interval_end_ms: i64,
}

#[derive(Debug, Clone)]
pub(super) struct LongTermProjectionIntervalSegment {
    pub(super) invocation_row_id: i64,
    pub(super) model_series_key: String,
    pub(super) upstream_series_key: String,
    pub(super) interval_start_ms: i64,
    pub(super) interval_end_ms: i64,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct LongTermProjectionIntervalStateRow {
    pub(super) invocation_row_id: i64,
    pub(super) model_series_key: String,
    pub(super) upstream_series_key: String,
    pub(super) interval_start_ms: i64,
    pub(super) interval_end_ms: i64,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct LongTermProjectionLegacyCompactRow {
    pub(super) invocation_row_id: i64,
    pub(super) model_series_key: Option<String>,
    pub(super) upstream_series_key: Option<String>,
    pub(super) interval_start_ms: i64,
    pub(super) interval_end_ms: i64,
}

#[derive(Debug)]
pub(super) struct LongTermProjectionEvent {
    pub(super) row_id: i64,
    pub(super) hourly: HashMap<(i64, String, String), LongTermBucket>,
    pub(super) daily: HashMap<(String, String, String), LongTermBucket>,
    pub(super) segments: Vec<LongTermProjectionIntervalSegment>,
    pub(super) bucket_dates: HashSet<String>,
}

impl LongTermProjectionRuntime {
    pub(crate) fn memory_estimate(&self) -> MemoryComponentEstimate {
        let interval_bytes = self
            .interval_index
            .iter()
            .map(|(key, union)| {
                key.bucket_key.capacity()
                    + key.dimension.capacity()
                    + key.series_key.capacity()
                    + union.intervals.len()
                        * (std::mem::size_of::<(i64, i64)>() + std::mem::size_of::<usize>() * 2)
            })
            .sum::<usize>();
        MemoryComponentEstimate {
            entries: self.interval_index.len(),
            bytes: interval_bytes
                .saturating_add(self.loaded_interval_dates.len().saturating_mul(64))
                .saturating_add(self.interval_index.capacity() * std::mem::size_of::<usize>() * 2),
            detail_items: self
                .interval_index
                .values()
                .map(|union| union.intervals.len())
                .sum(),
        }
    }

    pub(crate) fn health(&self) -> LongTermProjectionHealth {
        LongTermProjectionHealth {
            state: if self.state.is_empty() {
                "preparing".to_string()
            } else {
                self.state.clone()
            },
            cursor_row_id: self.cursor_row_id,
            dirty_bucket_count: self.dirty_bucket_count,
            pending_event_count: self.pending_event_count,
            last_flush_elapsed_ms: self.last_flush_elapsed_ms,
            last_flush_age_ms: self
                .last_flush_at
                .map(|last_flush_at| last_flush_at.elapsed().as_millis() as u64),
            last_repair_scope: self.last_repair_scope.clone(),
            last_defer_reason: self.last_defer_reason.clone(),
            last_error_kind: self.last_error_kind.clone(),
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct LongTermRollupRow {
    pub(super) bucket_or_date: String,
    pub(super) dimension: String,
    pub(super) series_key: String,
    pub(super) display_name: String,
    pub(super) reasoning_effort: String,
    pub(super) calls: i64,
    pub(super) token_total: i64,
    pub(super) token_samples: i64,
    pub(super) cost_total: f64,
    pub(super) cost_samples: i64,
    pub(super) usage_time_ms: f64,
    pub(super) usage_time_samples: i64,
    pub(super) wall_time_ms: f64,
    pub(super) wall_time_samples: i64,
    pub(super) output_tokens_total: i64,
    pub(super) stream_duration_ms: f64,
    pub(super) output_speed_samples: i64,
    pub(super) first_byte_sum_ms: f64,
    pub(super) first_byte_samples: i64,
    pub(super) response_sum_ms: f64,
    pub(super) response_samples: i64,
}

#[derive(Debug, Default, Clone)]
pub(super) struct LongTermAccumulator {
    pub(super) calls: i64,
    pub(super) token_total: i64,
    pub(super) token_samples: i64,
    pub(super) cost_total: f64,
    pub(super) cost_samples: i64,
    pub(super) usage_time_ms: f64,
    pub(super) usage_time_samples: i64,
    pub(super) output_tokens_total: i64,
    pub(super) stream_duration_ms: f64,
    pub(super) output_speed_samples: i64,
    pub(super) first_byte_sum_ms: f64,
    pub(super) first_byte_samples: i64,
    pub(super) response_sum_ms: f64,
    pub(super) response_samples: i64,
    pub(super) intervals: Vec<(i64, i64)>,
}

#[derive(Debug, Clone)]
pub(super) struct LongTermBucket {
    pub(super) bucket_start_epoch: i64,
    pub(super) dimension: String,
    pub(super) series_key: String,
    pub(super) display_name: String,
    pub(super) reasoning_effort: String,
    pub(super) stats_date: Option<String>,
    pub(super) accumulator: LongTermAccumulator,
}

impl LongTermAccumulator {
    pub(super) fn add_call(&mut self, row: &LongTermInvocationRow, interval: Option<(i64, i64)>) {
        self.calls += 1;
        if let Some(tokens) = row.total_tokens {
            self.token_total += tokens.max(0);
            self.token_samples += 1;
        }
        if let Some(cost) = row.cost.filter(|value| value.is_finite()) {
            self.cost_total += cost;
            self.cost_samples += 1;
        }
        if !is_success_status(row.status.as_deref(), row.error_message.as_deref()) {
            return;
        }
        if let Some(value) = row
            .t_total_ms
            .filter(|value| value.is_finite() && *value > 0.0)
        {
            self.usage_time_ms += value;
            self.usage_time_samples += 1;
        }
        if let (Some(output_tokens), Some(stream_duration_ms)) = (
            row.output_tokens,
            row.t_upstream_stream_ms
                .filter(|value| value.is_finite() && *value > 0.0),
        ) {
            self.output_tokens_total += output_tokens.max(0);
            self.stream_duration_ms += stream_duration_ms;
            self.output_speed_samples += 1;
        }
        if let Some(value) = crate::stats::resolve_first_response_byte_total_ms(
            row.t_req_read_ms,
            row.t_req_parse_ms,
            row.t_upstream_connect_ms,
            row.t_upstream_ttfb_ms,
        ) {
            self.first_byte_sum_ms += value;
            self.first_byte_samples += 1;
        }
        if let Some(value) = row
            .t_upstream_stream_ms
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            self.response_sum_ms += value;
            self.response_samples += 1;
        }
        if let Some((start, end)) = interval {
            self.intervals.push((start, end));
        }
    }

    pub(super) fn merge(&mut self, other: &Self) {
        self.calls += other.calls;
        self.token_total += other.token_total;
        self.token_samples += other.token_samples;
        self.cost_total += other.cost_total;
        self.cost_samples += other.cost_samples;
        self.usage_time_ms += other.usage_time_ms;
        self.usage_time_samples += other.usage_time_samples;
        self.output_tokens_total += other.output_tokens_total;
        self.stream_duration_ms += other.stream_duration_ms;
        self.output_speed_samples += other.output_speed_samples;
        self.first_byte_sum_ms += other.first_byte_sum_ms;
        self.first_byte_samples += other.first_byte_samples;
        self.response_sum_ms += other.response_sum_ms;
        self.response_samples += other.response_samples;
        self.intervals.extend_from_slice(&other.intervals);
    }

    pub(super) fn wall_time_ms(&self) -> f64 {
        union_interval_duration(&self.intervals) as f64
    }

    pub(super) fn wall_sample_count(&self) -> i64 {
        self.intervals.len() as i64
    }

    pub(super) fn add_interval(&mut self, interval: Option<(i64, i64)>) {
        if let Some(interval) = interval {
            self.intervals.push(interval);
        }
    }
}

impl LongTermMetrics {
    pub(super) fn from_accumulator(acc: &LongTermAccumulator) -> Self {
        Self {
            calls: acc.calls,
            tokens: (acc.token_samples > 0).then_some(acc.token_total),
            token_samples: acc.token_samples,
            cost: (acc.cost_samples > 0).then_some(acc.cost_total),
            cost_samples: acc.cost_samples,
            usage_time_ms: (acc.usage_time_samples > 0).then_some(acc.usage_time_ms),
            usage_time_samples: acc.usage_time_samples,
            wall_time_ms: (!acc.intervals.is_empty()).then_some(acc.wall_time_ms()),
            wall_time_samples: acc.wall_sample_count(),
            output_speed_tokens_per_second: (acc.output_speed_samples > 0
                && acc.stream_duration_ms > 0.0)
                .then_some(acc.output_tokens_total as f64 / (acc.stream_duration_ms / 1000.0)),
            output_speed_samples: acc.output_speed_samples,
            first_byte_ms: (acc.first_byte_samples > 0)
                .then_some(acc.first_byte_sum_ms / acc.first_byte_samples as f64),
            first_byte_samples: acc.first_byte_samples,
            response_ms: (acc.response_samples > 0)
                .then_some(acc.response_sum_ms / acc.response_samples as f64),
            response_samples: acc.response_samples,
        }
    }

    pub(super) fn from_rollup(row: &LongTermRollupRow) -> Self {
        Self {
            calls: row.calls,
            tokens: (row.token_samples > 0).then_some(row.token_total),
            token_samples: row.token_samples,
            cost: (row.cost_samples > 0).then_some(row.cost_total),
            cost_samples: row.cost_samples,
            usage_time_ms: (row.usage_time_samples > 0).then_some(row.usage_time_ms),
            usage_time_samples: row.usage_time_samples,
            wall_time_ms: (row.wall_time_samples > 0).then_some(row.wall_time_ms),
            wall_time_samples: row.wall_time_samples,
            output_speed_tokens_per_second: (row.output_speed_samples > 0
                && row.stream_duration_ms > 0.0)
                .then_some(row.output_tokens_total as f64 / (row.stream_duration_ms / 1000.0)),
            output_speed_samples: row.output_speed_samples,
            first_byte_ms: (row.first_byte_samples > 0)
                .then_some(row.first_byte_sum_ms / row.first_byte_samples as f64),
            first_byte_samples: row.first_byte_samples,
            response_ms: (row.response_samples > 0)
                .then_some(row.response_sum_ms / row.response_samples as f64),
            response_samples: row.response_samples,
        }
    }
}
