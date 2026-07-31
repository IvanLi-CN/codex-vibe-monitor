use super::*;

const LONG_TERM_TIMEZONE: &str = "Asia/Shanghai";
const LONG_TERM_STATE_ID: i64 = 1;
const LONG_TERM_STATUS_DISABLED: &str = "disabled";
const LONG_TERM_STATUS_PREPARING: &str = "preparing";
const LONG_TERM_STATUS_RUNNING: &str = "running";
const LONG_TERM_STATUS_READY: &str = "ready";
const LONG_TERM_STATUS_EMPTY: &str = "empty";
const LONG_TERM_STATUS_ERROR: &str = "error";
const LONG_TERM_TERMINAL_PROOF_UNAVAILABLE_ERROR: &str =
    "terminal integrity proof reconciliation is incomplete";
const LONG_TERM_ATTEMPT_ARCHIVE_UNAVAILABLE_ERROR: &str =
    "attempt archive is unavailable for long-term account attribution";
const LONG_TERM_OTHER_KEY: &str = "other";
const LONG_TERM_OTHER_NAME: &str = "其他";
const LONG_TERM_HOUR_MS: i64 = 60 * 60 * 1000;
const LONG_TERM_INTEGRITY_AUDIT_INTERVAL_SECS: i64 = 60 * 60;
const LONG_TERM_REPAIR_BACKOFF_SECS: [i64; 4] = [60, 5 * 60, 15 * 60, 60 * 60];
const LONG_TERM_REFRESH_LOCK_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_millis(250),
    Duration::from_secs(1),
    Duration::from_secs(3),
];
pub(crate) const LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET: &str = "long_term_usage_stats";

static LONG_TERM_REFRESH_LOCK: once_cell::sync::Lazy<Mutex<()>> =
    once_cell::sync::Lazy::new(|| Mutex::new(()));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LongTermRange {
    Seven,
    Thirty,
    OneEighty,
    ThreeSixtyFive,
}

impl LongTermRange {
    fn parse(raw: Option<&str>) -> Option<Self> {
        match raw.unwrap_or("7d") {
            "7d" => Some(Self::Seven),
            "30d" => Some(Self::Thirty),
            "180d" => Some(Self::OneEighty),
            "365d" => Some(Self::ThreeSixtyFive),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Seven => "7d",
            Self::Thirty => "30d",
            Self::OneEighty => "180d",
            Self::ThreeSixtyFive => "365d",
        }
    }

    fn days(self) -> i64 {
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

fn parse_long_term_series_query(uri: &Uri) -> LongTermSeriesQuery {
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
struct LongTermStateRow {
    status: String,
    statistics_start_date: Option<String>,
    integrity_source_start_date: Option<String>,
    processed_rows: i64,
    total_rows: i64,
    last_error: Option<String>,
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
    interval_index: HashMap<LongTermProjectionIntervalKey, LongTermProjectionIntervalUnion>,
    loaded_interval_dates: HashSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LongTermProjectionIntervalKey {
    bucket_kind: &'static str,
    bucket_key: String,
    dimension: String,
    series_key: String,
}

#[derive(Debug, Clone, Default)]
struct LongTermProjectionIntervalUnion {
    intervals: BTreeMap<i64, i64>,
    duration_ms: i64,
    sample_count: i64,
}

impl LongTermProjectionIntervalUnion {
    fn add(&mut self, mut start_ms: i64, mut end_ms: i64) {
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
struct LongTermProjectionIntervalRow {
    bucket_kind: String,
    bucket_key: String,
    dimension: String,
    series_key: String,
    interval_start_ms: i64,
    interval_end_ms: i64,
}

#[derive(Debug, Clone)]
struct LongTermProjectionIntervalSegment {
    key: LongTermProjectionIntervalKey,
    bucket_date: String,
    invocation_row_id: i64,
    interval_start_ms: i64,
    interval_end_ms: i64,
}

#[derive(Debug)]
struct LongTermProjectionEvent {
    row_id: i64,
    hourly: HashMap<(i64, String, String), LongTermBucket>,
    daily: HashMap<(String, String, String), LongTermBucket>,
    segments: Vec<LongTermProjectionIntervalSegment>,
    bucket_dates: HashSet<String>,
}

impl LongTermProjectionRuntime {
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

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct LongTermIntegrityTotals {
    calls: i64,
    token_total: i64,
    cost_total: f64,
}

#[derive(Debug, Clone)]
struct LongTermIntegrityOracle {
    date: NaiveDate,
    daily: LongTermIntegrityTotals,
    hourly: HashMap<i64, LongTermIntegrityTotals>,
}

#[derive(Debug, Clone)]
struct LongTermIntegrityMismatch {
    date: NaiveDate,
    expected: LongTermIntegrityTotals,
    observed: LongTermIntegrityTotals,
    reason: String,
}

#[derive(Debug, Clone, FromRow)]
struct LongTermIntegrityHourRow {
    bucket_start_epoch: i64,
    calls: i64,
    token_total: i64,
    cost_total: f64,
}

#[derive(Debug, Clone, FromRow)]
struct LongTermInvocationRow {
    id: i64,
    invoke_id: Option<String>,
    occurred_at: String,
    status: Option<String>,
    model: Option<String>,
    request_model: Option<String>,
    response_model: Option<String>,
    reasoning_effort: Option<String>,
    upstream_account_id: Option<i64>,
    upstream_account_kind: Option<String>,
    upstream_account_name: Option<String>,
    total_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cost: Option<f64>,
    t_total_ms: Option<f64>,
    t_req_read_ms: Option<f64>,
    t_req_parse_ms: Option<f64>,
    t_upstream_connect_ms: Option<f64>,
    t_upstream_ttfb_ms: Option<f64>,
    t_upstream_stream_ms: Option<f64>,
    error_message: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
struct LongTermArchiveAttemptRow {
    invoke_id: String,
    occurred_at: String,
    upstream_account_id: Option<i64>,
}

#[derive(Debug, Clone, FromRow)]
struct LongTermSourceTimingRow {
    invoke_id: Option<String>,
    occurred_at: String,
    t_total_ms: Option<f64>,
}

#[derive(Debug, Clone, FromRow)]
struct LongTermAttemptArchivePath {
    file_path: String,
    sha256: String,
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
}

async fn load_long_term_archive_attempt_accounts(
    pool: &Pool<Sqlite>,
    date_range: Option<(NaiveDate, NaiveDate)>,
) -> Result<(HashMap<(String, String), i64>, HashSet<(String, String)>)> {
    let paths = match sqlx::query_as::<_, LongTermAttemptArchivePath>(
        "SELECT file_path, sha256, coverage_start_at, coverage_end_at FROM archive_batches WHERE dataset = 'pool_upstream_request_attempts' AND status = ?1 ORDER BY month_key ASC, created_at ASC, id ASC",
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_all(pool)
    .await
    {
        Ok(paths) => paths,
        Err(error) if error.to_string().contains("no such table") => {
            return Ok((HashMap::new(), HashSet::new()));
        }
        Err(error) => return Err(error.into()),
    };
    let mut accounts = HashMap::new();
    let mut consumed_archives = HashSet::new();
    for archive_path in paths.into_iter().filter(|path| {
        date_range.is_none_or(|(start, end)| {
            let Some(path_start) = path
                .coverage_start_at
                .as_deref()
                .and_then(long_term_archive_end_date)
            else {
                return true;
            };
            let path_end = path
                .coverage_end_at
                .as_deref()
                .and_then(long_term_archive_end_date)
                .unwrap_or(path_start);
            path_end >= start && path_start <= end
        })
    }) {
        let attempt_archive = open_pool_upstream_request_attempt_archive_batch_pool(
            &ArchiveBatchPathRow::from_file_path(archive_path.file_path.clone()),
            "long-term-stats-attempt-fallback",
        )
        .await
        .with_context(|| {
            format!(
                "{LONG_TERM_ATTEMPT_ARCHIVE_UNAVAILABLE_ERROR}: {}",
                archive_path.file_path
            )
        })?;
        let Some((archive_pool, cleanup)) = attempt_archive else {
            bail!(
                "{LONG_TERM_ATTEMPT_ARCHIVE_UNAVAILABLE_ERROR}: {}",
                archive_path.file_path,
            );
        };
        let mut query = String::from(
            r#"
            SELECT invoke_id, occurred_at, upstream_account_id
            FROM pool_upstream_request_attempts
            WHERE upstream_account_id IS NOT NULL
            "#,
        );
        if date_range.is_some() {
            query.push_str(" AND occurred_at >= ?1 AND occurred_at < ?2");
        }
        query.push_str(" ORDER BY id ASC");
        let mut statement = sqlx::query_as::<_, LongTermArchiveAttemptRow>(&query);
        if let Some((start, end)) = date_range {
            statement = statement
                .bind(format!("{start} 00:00:00"))
                .bind(format!("{} 00:00:00", end.succ_opt().unwrap_or(end)));
        }
        let rows = statement.fetch_all(&archive_pool).await;
        archive_pool.close().await;
        drop(cleanup);
        match rows {
            Ok(rows) => {
                for row in rows {
                    if let Some(account_id) = row.upstream_account_id {
                        accounts.insert((row.invoke_id, row.occurred_at), account_id);
                    }
                }
                consumed_archives.insert((archive_path.file_path, archive_path.sha256));
            }
            Err(error) if error.to_string().contains("no such table") => {
                consumed_archives.insert((archive_path.file_path, archive_path.sha256));
            }
            Err(error) => {
                bail!(
                    "{LONG_TERM_ATTEMPT_ARCHIVE_UNAVAILABLE_ERROR}: {}: {error}",
                    archive_path.file_path
                );
            }
        }
    }
    Ok((accounts, consumed_archives))
}

fn hydrate_long_term_archive_attempt_account(
    row: &mut LongTermInvocationRow,
    attempt_accounts: &HashMap<(String, String), i64>,
) {
    if row.upstream_account_id.is_none()
        && let Some(invoke_id) = row.invoke_id.as_ref()
        && let Some(account_id) =
            attempt_accounts.get(&(invoke_id.clone(), row.occurred_at.clone()))
    {
        row.upstream_account_id = Some(*account_id);
    }
}

async fn long_term_archive_invocation_query(pool: &Pool<Sqlite>) -> Result<String> {
    long_term_archive_invocation_query_with_range(pool, false).await
}

async fn long_term_archive_invocation_query_for_range(pool: &Pool<Sqlite>) -> Result<String> {
    long_term_archive_invocation_query_with_range(pool, true).await
}

async fn long_term_archive_invocation_query_with_range(
    pool: &Pool<Sqlite>,
    bounded_to_target_date: bool,
) -> Result<String> {
    let columns = load_archive_table_columns(pool, "codex_invocations").await?;
    let select = |column: &str| long_term_legacy_select_expr(&columns, column);
    let status_column = if columns.contains("status") {
        "status"
    } else {
        "NULL"
    };
    let payload = if columns.contains("payload") {
        "payload"
    } else {
        "NULL"
    };
    let request_model = format!(
        "CASE WHEN json_valid({payload}) THEN NULLIF(TRIM(CAST(json_extract({payload}, '$.requestModel') AS TEXT)), '') END"
    );
    let response_model = format!(
        "CASE WHEN json_valid({payload}) THEN NULLIF(TRIM(CAST(json_extract({payload}, '$.responseModel') AS TEXT)), '') END"
    );
    let reasoning_effort = format!(
        "CASE WHEN json_valid({payload}) THEN NULLIF(TRIM(CAST(json_extract({payload}, '$.reasoningEffort') AS TEXT)), '') END"
    );
    let payload_upstream_account_id = format!(
        "CASE WHEN json_valid({payload}) THEN CAST(json_extract({payload}, '$.upstreamAccountId') AS INTEGER) END"
    );
    let upstream_account_id = if columns.contains("upstream_account_id") {
        format!("COALESCE(upstream_account_id, {payload_upstream_account_id})")
    } else {
        payload_upstream_account_id
    };
    let t_total_ms_column = if columns.contains("t_total_ms") {
        "t_total_ms"
    } else {
        "NULL"
    };
    let range_filter = bounded_to_target_date.then(|| {
        format!(
            r#"
        AND occurred_at < ?1
        AND (
            occurred_at >= ?2
            OR (
                {t_total_ms_column} IS NOT NULL
                AND {t_total_ms_column} > 0
                AND julianday(occurred_at) + {t_total_ms_column} / 86400000.0 >= julianday(?2)
            )
        )
            "#,
        )
    });
    Ok(format!(
        r#"
        SELECT
            id,
            {invoke_id},
            occurred_at,
            {status},
            {model},
            {request_model} AS request_model,
            {response_model} AS response_model,
            {reasoning_effort} AS reasoning_effort,
            {upstream_account_id} AS upstream_account_id,
            NULL AS upstream_account_kind,
            NULL AS upstream_account_name,
            {total_tokens},
            {output_tokens},
            {cost},
            {t_total_ms},
            {t_req_read_ms},
            {t_req_parse_ms},
            {t_upstream_connect_ms},
            {t_upstream_ttfb_ms},
            {t_upstream_stream_ms},
            {error_message}
        FROM codex_invocations
        WHERE LOWER(TRIM(COALESCE({status_column}, ''))) NOT IN ('running', 'pending')
        {range_filter}
        ORDER BY occurred_at ASC, id ASC
        "#,
        invoke_id = select("invoke_id"),
        status = select("status"),
        status_column = status_column,
        model = select("model"),
        request_model = request_model,
        response_model = response_model,
        reasoning_effort = reasoning_effort,
        upstream_account_id = upstream_account_id,
        total_tokens = select("total_tokens"),
        output_tokens = select("output_tokens"),
        cost = select("cost"),
        t_total_ms = select("t_total_ms"),
        t_req_read_ms = select("t_req_read_ms"),
        t_req_parse_ms = select("t_req_parse_ms"),
        t_upstream_connect_ms = select("t_upstream_connect_ms"),
        t_upstream_ttfb_ms = select("t_upstream_ttfb_ms"),
        t_upstream_stream_ms = select("t_upstream_stream_ms"),
        error_message = select("error_message"),
        range_filter = range_filter.as_deref().unwrap_or_default(),
    ))
}

fn long_term_legacy_select_expr(columns: &HashSet<String>, column: &str) -> String {
    format!(
        "{} AS {column}",
        long_term_legacy_column_expr(columns, column)
    )
}

fn long_term_legacy_column_expr(columns: &HashSet<String>, column: &str) -> String {
    if columns.contains(column) {
        column.to_string()
    } else {
        "NULL".to_string()
    }
}

#[derive(Debug, Clone, FromRow)]
struct LongTermRollupRow {
    bucket_or_date: String,
    dimension: String,
    series_key: String,
    display_name: String,
    reasoning_effort: String,
    calls: i64,
    token_total: i64,
    token_samples: i64,
    cost_total: f64,
    cost_samples: i64,
    usage_time_ms: f64,
    usage_time_samples: i64,
    wall_time_ms: f64,
    wall_time_samples: i64,
    output_tokens_total: i64,
    stream_duration_ms: f64,
    output_speed_samples: i64,
    first_byte_sum_ms: f64,
    first_byte_samples: i64,
    response_sum_ms: f64,
    response_samples: i64,
}

#[derive(Debug, Default, Clone)]
struct LongTermAccumulator {
    calls: i64,
    token_total: i64,
    token_samples: i64,
    cost_total: f64,
    cost_samples: i64,
    usage_time_ms: f64,
    usage_time_samples: i64,
    output_tokens_total: i64,
    stream_duration_ms: f64,
    output_speed_samples: i64,
    first_byte_sum_ms: f64,
    first_byte_samples: i64,
    response_sum_ms: f64,
    response_samples: i64,
    intervals: Vec<(i64, i64)>,
}

#[derive(Debug, Clone)]
struct LongTermBucket {
    bucket_start_epoch: i64,
    dimension: String,
    series_key: String,
    display_name: String,
    reasoning_effort: String,
    stats_date: Option<String>,
    accumulator: LongTermAccumulator,
}

impl LongTermAccumulator {
    fn add_call(&mut self, row: &LongTermInvocationRow, interval: Option<(i64, i64)>) {
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

    fn merge(&mut self, other: &Self) {
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

    fn wall_time_ms(&self) -> f64 {
        union_interval_duration(&self.intervals) as f64
    }

    fn wall_sample_count(&self) -> i64 {
        self.intervals.len() as i64
    }

    fn add_interval(&mut self, interval: Option<(i64, i64)>) {
        if let Some(interval) = interval {
            self.intervals.push(interval);
        }
    }
}

impl LongTermMetrics {
    fn from_accumulator(acc: &LongTermAccumulator) -> Self {
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

    fn from_rollup(row: &LongTermRollupRow) -> Self {
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

pub(crate) fn ensure_long_term_stats_schema_sql() -> &'static str {
    "long_term_usage_hourly, long_term_usage_daily, long_term_stats_state and long_term_stats_repair_queue"
}

pub(crate) async fn ensure_long_term_stats_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_usage_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            dimension TEXT NOT NULL,
            series_key TEXT NOT NULL,
            display_name TEXT NOT NULL,
            reasoning_effort TEXT NOT NULL DEFAULT '',
            calls INTEGER NOT NULL DEFAULT 0,
            token_total INTEGER NOT NULL DEFAULT 0,
            token_samples INTEGER NOT NULL DEFAULT 0,
            cost_total REAL NOT NULL DEFAULT 0,
            cost_samples INTEGER NOT NULL DEFAULT 0,
            usage_time_ms REAL NOT NULL DEFAULT 0,
            usage_time_samples INTEGER NOT NULL DEFAULT 0,
            wall_time_ms REAL NOT NULL DEFAULT 0,
            wall_time_samples INTEGER NOT NULL DEFAULT 0,
            output_tokens_total INTEGER NOT NULL DEFAULT 0,
            stream_duration_ms REAL NOT NULL DEFAULT 0,
            output_speed_samples INTEGER NOT NULL DEFAULT 0,
            first_byte_sum_ms REAL NOT NULL DEFAULT 0,
            first_byte_samples INTEGER NOT NULL DEFAULT 0,
            response_sum_ms REAL NOT NULL DEFAULT 0,
            response_samples INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, dimension, series_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long_term_usage_hourly table")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_usage_daily (
            stats_date TEXT NOT NULL,
            dimension TEXT NOT NULL,
            series_key TEXT NOT NULL,
            display_name TEXT NOT NULL,
            reasoning_effort TEXT NOT NULL DEFAULT '',
            calls INTEGER NOT NULL DEFAULT 0,
            token_total INTEGER NOT NULL DEFAULT 0,
            token_samples INTEGER NOT NULL DEFAULT 0,
            cost_total REAL NOT NULL DEFAULT 0,
            cost_samples INTEGER NOT NULL DEFAULT 0,
            usage_time_ms REAL NOT NULL DEFAULT 0,
            usage_time_samples INTEGER NOT NULL DEFAULT 0,
            wall_time_ms REAL NOT NULL DEFAULT 0,
            wall_time_samples INTEGER NOT NULL DEFAULT 0,
            output_tokens_total INTEGER NOT NULL DEFAULT 0,
            stream_duration_ms REAL NOT NULL DEFAULT 0,
            output_speed_samples INTEGER NOT NULL DEFAULT 0,
            first_byte_sum_ms REAL NOT NULL DEFAULT 0,
            first_byte_samples INTEGER NOT NULL DEFAULT 0,
            response_sum_ms REAL NOT NULL DEFAULT 0,
            response_samples INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (stats_date, dimension, series_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long_term_usage_daily table")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_stats_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            status TEXT NOT NULL DEFAULT 'preparing',
            statistics_start_date TEXT,
            integrity_source_start_date TEXT,
            integrity_source_pending_start_date TEXT,
            processed_rows INTEGER NOT NULL DEFAULT 0,
            total_rows INTEGER NOT NULL DEFAULT 0,
            last_error TEXT,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long_term_stats_state table")?;
    sqlx::query("INSERT OR IGNORE INTO long_term_stats_state (id, status) VALUES (?1, ?2)")
        .bind(LONG_TERM_STATE_ID)
        .bind(LONG_TERM_STATUS_PREPARING)
        .execute(pool)
        .await
        .context("failed to seed long term stats state")?;
    let state_columns = load_sqlite_table_columns(pool, "long_term_stats_state").await?;
    if !state_columns.contains("last_integrity_audit_at") {
        sqlx::query("ALTER TABLE long_term_stats_state ADD COLUMN last_integrity_audit_at TEXT")
            .execute(pool)
            .await
            .context("failed to add long term integrity audit timestamp")?;
    }
    if !state_columns.contains("integrity_source_start_date") {
        sqlx::query(
            "ALTER TABLE long_term_stats_state ADD COLUMN integrity_source_start_date TEXT",
        )
        .execute(pool)
        .await
        .context("failed to add long term integrity source boundary")?;
    }
    if !state_columns.contains("integrity_source_pending_start_date") {
        sqlx::query(
            "ALTER TABLE long_term_stats_state ADD COLUMN integrity_source_pending_start_date TEXT",
        )
        .execute(pool)
        .await
        .context("failed to add pending long term integrity source boundary")?;
    }
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_stats_repair_queue (
            stats_date TEXT PRIMARY KEY,
            expected_calls INTEGER NOT NULL,
            expected_token_total INTEGER NOT NULL,
            expected_cost_total REAL NOT NULL,
            observed_calls INTEGER NOT NULL,
            observed_token_total INTEGER NOT NULL,
            observed_cost_total REAL NOT NULL,
            attempts INTEGER NOT NULL DEFAULT 0,
            next_retry_at TEXT NOT NULL DEFAULT (datetime('now')),
            last_error TEXT NOT NULL,
            detected_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long term integrity repair queue")?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_long_term_stats_repair_queue_due ON long_term_stats_repair_queue (next_retry_at, stats_date)",
    )
    .execute(pool)
    .await
    .context("failed to ensure long term integrity repair queue index")?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_long_term_usage_daily_dimension_date ON long_term_usage_daily (dimension, stats_date)",
    )
    .execute(pool)
    .await
    .context("failed to ensure long term daily index")?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_long_term_usage_hourly_dimension_bucket ON long_term_usage_hourly (dimension, bucket_start_epoch)",
    )
    .execute(pool)
    .await
    .context("failed to ensure long term hourly index")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS hourly_rollup_archive_replay (
            target TEXT NOT NULL,
            dataset TEXT NOT NULL,
            file_path TEXT NOT NULL,
            replayed_at TEXT NOT NULL DEFAULT (datetime('now')),
            archive_sha256 TEXT,
            PRIMARY KEY (target, dataset, file_path)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long term archive replay marker table")?;
    let replay_columns = load_sqlite_table_columns(pool, "hourly_rollup_archive_replay").await?;
    if !replay_columns.contains("archive_sha256") {
        sqlx::query("ALTER TABLE hourly_rollup_archive_replay ADD COLUMN archive_sha256 TEXT")
            .execute(pool)
            .await
            .context("failed to add archive hash to replay markers")?;
    }
    ensure_long_term_projection_schema(pool).await?;
    ensure_long_term_projection_correction_trigger(pool).await?;
    ensure_long_term_projection_archive_trigger(pool).await?;
    Ok(())
}

const LONG_TERM_PROJECTION_CONSUMER: &str = "long_term_v1";
const LONG_TERM_PROJECTION_FLUSH_INTERVAL: Duration = Duration::from_secs(60);
const LONG_TERM_PROJECTION_DAILY_VERIFY_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const LONG_TERM_PROJECTION_MAX_BUCKETS_PER_FLUSH: i64 = 1;
const LONG_TERM_PROJECTION_MAX_EVENTS_PER_FLUSH: i64 = 2_000;

async fn ensure_long_term_projection_correction_trigger(pool: &Pool<Sqlite>) -> Result<()> {
    let invocation_table_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'codex_invocations')",
    )
    .fetch_one(pool)
    .await?;
    if invocation_table_exists == 0 {
        return Ok(());
    }

    // Corrections originate in several write-side workers. Queue every local day touched by the
    // old or new interval so a later projection pass can replace only the affected buckets.
    sqlx::query(
        r#"
        CREATE TRIGGER IF NOT EXISTS long_term_projection_invocation_correction
        AFTER UPDATE OF
          source, status, occurred_at, model, payload,
          input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens,
          cost, t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms,
          t_upstream_ttfb_ms, t_upstream_stream_ms, error_message
        ON codex_invocations
        BEGIN
          INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason)
          WITH RECURSIVE affected_dates(bucket_date, end_date) AS (
            SELECT
              date(OLD.occurred_at),
              date(julianday(OLD.occurred_at) + MAX(COALESCE(OLD.t_total_ms, 0), 0) / 86400000.0)
            WHERE OLD.occurred_at IS NOT NULL AND TRIM(OLD.occurred_at) <> ''
            UNION ALL
            SELECT
              date(NEW.occurred_at),
              date(julianday(NEW.occurred_at) + MAX(COALESCE(NEW.t_total_ms, 0), 0) / 86400000.0)
            WHERE NEW.occurred_at IS NOT NULL AND TRIM(NEW.occurred_at) <> ''
            UNION ALL
            SELECT date(bucket_date, '+1 day'), end_date
            FROM affected_dates
            WHERE bucket_date < end_date
          )
          SELECT DISTINCT bucket_date, 'invocation_correction'
          FROM affected_dates
          WHERE bucket_date IS NOT NULL
          ON CONFLICT(bucket_date) DO UPDATE SET
            repair_reason = excluded.repair_reason,
            updated_at = datetime('now');
        END
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection correction trigger")?;
    Ok(())
}

async fn ensure_long_term_projection_archive_trigger(pool: &Pool<Sqlite>) -> Result<()> {
    let archive_table_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'archive_batches')",
    )
    .fetch_one(pool)
    .await?;
    if archive_table_exists == 0 {
        return Ok(());
    }

    // Archive writes and rewrites are source changes for durable long-term rollups. Invocation
    // archives provide the terminal facts; attempt archives provide a later account-attribution
    // fallback. Both must invalidate the same target dates. Recreate these triggers so upgrades
    // do not retain a prior definition that only observed invocation archives.
    sqlx::query("DROP TRIGGER IF EXISTS long_term_projection_archive_insert")
        .execute(pool)
        .await?;
    sqlx::query("DROP TRIGGER IF EXISTS long_term_projection_archive_update")
        .execute(pool)
        .await?;

    sqlx::query(
        r#"
        CREATE TRIGGER long_term_projection_archive_insert
        AFTER INSERT ON archive_batches
        WHEN NEW.dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
          AND NEW.status = 'completed'
          AND NEW.coverage_start_at IS NOT NULL
          AND NEW.coverage_end_at IS NOT NULL
        BEGIN
          INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason)
          WITH RECURSIVE covered_dates(bucket_date, end_date) AS (
            SELECT date(NEW.coverage_start_at), date(NEW.coverage_end_at)
            UNION ALL
            SELECT date(bucket_date, '+1 day'), end_date
            FROM covered_dates
            WHERE bucket_date < end_date
          )
          SELECT DISTINCT bucket_date, 'archive_source_changed'
          FROM covered_dates
          WHERE bucket_date IS NOT NULL
          ON CONFLICT(bucket_date) DO UPDATE SET
            repair_reason = excluded.repair_reason,
            next_attempt_at = NULL,
            updated_at = datetime('now');
        END
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection archive insert trigger")?;

    sqlx::query(
        r#"
        CREATE TRIGGER long_term_projection_archive_update
        AFTER UPDATE OF dataset, status, file_path, sha256, coverage_start_at, coverage_end_at, historical_rollups_materialized_at ON archive_batches
        WHEN (
              NEW.dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
              AND NEW.status = 'completed'
              AND NEW.coverage_start_at IS NOT NULL
              AND NEW.coverage_end_at IS NOT NULL
            )
            OR (
              OLD.dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
              AND OLD.status = 'completed'
              AND OLD.coverage_start_at IS NOT NULL
              AND OLD.coverage_end_at IS NOT NULL
            )
        BEGIN
          INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason)
          WITH RECURSIVE coverage_ranges(start_date, end_date) AS (
            SELECT date(NEW.coverage_start_at), date(NEW.coverage_end_at)
            WHERE NEW.dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
              AND NEW.status = 'completed'
              AND NEW.coverage_start_at IS NOT NULL
              AND NEW.coverage_end_at IS NOT NULL
            UNION ALL
            SELECT date(OLD.coverage_start_at), date(OLD.coverage_end_at)
            WHERE OLD.dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
              AND OLD.status = 'completed'
              AND OLD.coverage_start_at IS NOT NULL
              AND OLD.coverage_end_at IS NOT NULL
          ), covered_dates(bucket_date, end_date) AS (
            SELECT start_date, end_date FROM coverage_ranges
            UNION ALL
            SELECT date(bucket_date, '+1 day'), end_date
            FROM covered_dates
            WHERE bucket_date < end_date
          )
          SELECT DISTINCT bucket_date, 'archive_source_changed'
          FROM covered_dates
          WHERE bucket_date IS NOT NULL
          ON CONFLICT(bucket_date) DO UPDATE SET
            repair_reason = excluded.repair_reason,
            next_attempt_at = NULL,
            updated_at = datetime('now');
        END
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection archive update trigger")?;
    Ok(())
}

#[derive(Debug, Clone, FromRow)]
struct LongTermProjectionCursorRow {
    cursor_row_id: i64,
}

pub(crate) fn spawn_long_term_projection_supervisor(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut flush_ticker = interval(LONG_TERM_PROJECTION_FLUSH_INTERVAL);
        flush_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        flush_ticker.tick().await;
        let mut daily_verify_ticker = interval(LONG_TERM_PROJECTION_DAILY_VERIFY_INTERVAL);
        daily_verify_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        daily_verify_ticker.tick().await;

        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = state.terminal_projection_hub.wait_for_persisted_work() => {
                    debug!(projection = "long_term", trigger = "terminal_p1_ack", "long-term projection marked dirty by terminal persistence");
                }
                _ = flush_ticker.tick() => {
                    if let Err(error) = flush_long_term_projection(&state, "terminal_deadline").await {
                        mark_long_term_projection_failure(&state, &error).await;
                        warn!(error = %error, projection = "long_term", trigger = "terminal_deadline", "long-term projection flush failed");
                    }
                }
                _ = daily_verify_ticker.tick() => {
                    if let Err(error) = queue_long_term_projection_daily_verify(&state.pool).await {
                        warn!(error = %error, projection = "long_term", trigger = "daily_verify", "failed to queue long-term projection daily verification");
                    } else if let Err(error) = flush_long_term_projection(&state, "daily_verify").await {
                        mark_long_term_projection_failure(&state, &error).await;
                        warn!(error = %error, projection = "long_term", trigger = "daily_verify", "long-term projection daily verification failed");
                    }
                }
            }
        }
    })
}

async fn mark_long_term_projection_failure(state: &AppState, error: &anyhow::Error) {
    let message = error.to_string().to_ascii_lowercase();
    let error_kind =
        if message.contains("database is locked") || message.contains("database is busy") {
            "sqlite_lock"
        } else if message.contains("source coverage incomplete") {
            "source_coverage"
        } else {
            "builder_error"
        };
    let mut runtime = state.long_term_projection_runtime.lock().await;
    runtime.state = "dirty_last_good".to_string();
    runtime.last_error_kind = Some(error_kind.to_string());
}

async fn queue_long_term_projection_daily_verify(pool: &Pool<Sqlite>) -> Result<()> {
    let today = Utc::now().with_timezone(&Shanghai).date_naive().to_string();
    queue_long_term_projection_repairs(pool, &[today], "daily_verify").await?;
    Ok(())
}

fn long_term_projection_open_baseline_dates() -> Vec<String> {
    let today = Utc::now().with_timezone(&Shanghai).date_naive();
    let yesterday = today.pred_opt().unwrap_or(today);
    vec![yesterday.to_string(), today.to_string()]
}

async fn queue_long_term_projection_repairs(
    pool: &Pool<Sqlite>,
    dates: &[String],
    repair_reason: &str,
) -> Result<()> {
    if dates.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for date in dates {
        sqlx::query(
            "INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason) VALUES (?1, ?2) ON CONFLICT(bucket_date) DO UPDATE SET repair_reason = excluded.repair_reason, next_attempt_at = NULL, updated_at = datetime('now')",
        )
        .bind(date)
        .bind(repair_reason)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn defer_long_term_projection_repair(pool: &Pool<Sqlite>, bucket_date: &str) -> Result<()> {
    sqlx::query(
        "UPDATE long_term_projection_dirty_buckets SET next_attempt_at = datetime('now', '+5 minutes'), updated_at = datetime('now') WHERE bucket_date = ?1",
    )
    .bind(bucket_date)
    .execute(pool)
    .await?;
    Ok(())
}

async fn load_long_term_projection_ready_dates(
    pool: &Pool<Sqlite>,
    dates: &HashSet<String>,
) -> Result<HashSet<String>> {
    if dates.is_empty() {
        return Ok(HashSet::new());
    }
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT state.bucket_date FROM long_term_projection_bucket_state state WHERE state.interval_baseline_ready = 1 AND NOT EXISTS (SELECT 1 FROM long_term_projection_dirty_buckets dirty WHERE dirty.bucket_date = state.bucket_date) AND state.bucket_date IN (",
    );
    let mut separated = builder.separated(", ");
    for date in dates {
        separated.push_bind(date);
    }
    separated.push_unseparated(")");
    Ok(builder
        .build_query_scalar::<String>()
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect())
}

async fn load_long_term_projection_terminal_rows(
    pool: &Pool<Sqlite>,
    cursor: i64,
) -> Result<Vec<LongTermInvocationRow>> {
    let has_attempt_table = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'pool_upstream_request_attempts')",
    )
    .fetch_one(pool)
    .await?
        != 0;
    let upstream_account_sql = if has_attempt_table {
        "COALESCE(CASE WHEN json_valid(inv.payload) THEN CAST(json_extract(inv.payload, '$.upstreamAccountId') AS INTEGER) END, (SELECT attempt.upstream_account_id FROM pool_upstream_request_attempts attempt WHERE attempt.invoke_id = inv.invoke_id AND attempt.occurred_at = inv.occurred_at AND attempt.upstream_account_id IS NOT NULL ORDER BY attempt.attempt_index DESC, attempt.id DESC LIMIT 1))"
    } else {
        "CASE WHEN json_valid(inv.payload) THEN CAST(json_extract(inv.payload, '$.upstreamAccountId') AS INTEGER) END"
    };
    let query = format!(
        r#"
        SELECT inv.id, inv.invoke_id, inv.occurred_at, inv.status, inv.model,
          CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.requestModel') AS TEXT)), '') END AS request_model,
          CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.responseModel') AS TEXT)), '') END AS response_model,
          CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.reasoningEffort') AS TEXT)), '') END AS reasoning_effort,
          {upstream_account_sql} AS upstream_account_id,
          NULL AS upstream_account_kind, NULL AS upstream_account_name,
          inv.total_tokens, inv.output_tokens, inv.cost, inv.t_total_ms,
          inv.t_req_read_ms, inv.t_req_parse_ms, inv.t_upstream_connect_ms,
          inv.t_upstream_ttfb_ms, inv.t_upstream_stream_ms, inv.error_message
        FROM codex_invocations inv
        WHERE inv.id > ?1
          AND LOWER(TRIM(COALESCE(inv.status, ''))) NOT IN ('running', 'pending')
        ORDER BY inv.id ASC
        LIMIT ?2
        "#,
    );
    Ok(sqlx::query_as::<_, LongTermInvocationRow>(&query)
        .bind(cursor)
        .bind(LONG_TERM_PROJECTION_MAX_EVENTS_PER_FLUSH)
        .fetch_all(pool)
        .await?)
}

fn build_long_term_projection_event(row: &LongTermInvocationRow) -> LongTermProjectionEvent {
    let mut hourly = HashMap::new();
    let mut daily = HashMap::new();
    let mut statistics_start = None;
    accumulate_long_term_invocation(row, &mut hourly, &mut daily, &mut statistics_start);
    let bucket_dates = daily
        .keys()
        .map(|(bucket_date, _, _)| bucket_date.clone())
        .collect::<HashSet<_>>();
    let segments = collect_long_term_projection_interval_segments(&hourly, &daily, row.id);
    LongTermProjectionEvent {
        row_id: row.id,
        hourly,
        daily,
        segments,
        bucket_dates,
    }
}

async fn invalidate_long_term_projection_interval_cache(state: &AppState) {
    let mut runtime = state.long_term_projection_runtime.lock().await;
    runtime.interval_index.clear();
    runtime.loaded_interval_dates.clear();
}

async fn flush_long_term_projection(state: &AppState, trigger: &'static str) -> Result<()> {
    let gate = crate::db_pressure::global_db_pressure_gate();
    let _permit = match gate.try_begin_background("long_term_projection_flush") {
        Ok(permit) => permit,
        Err(reason) => {
            let mut runtime = state.long_term_projection_runtime.lock().await;
            runtime.state = "deferred".to_string();
            runtime.last_defer_reason = Some("writer_pressure".to_string());
            debug!(projection = "long_term", trigger, gate_outcome = "deferred", defer_reason = "writer_pressure", reason = %reason, "long-term projection flush deferred by database pressure gate");
            return Ok(());
        }
    };

    let started = Instant::now();
    let mut cursor = load_long_term_projection_cursor(&state.pool).await?;
    let state_row = load_long_term_state(&state.pool).await?;
    let mut baseline_cursor = None;
    if !matches!(
        state_row.status.as_str(),
        LONG_TERM_STATUS_READY | LONG_TERM_STATUS_EMPTY
    ) && !long_term_rollups_exist(&state.pool).await?
    {
        // First installation retains the existing preparing/full materialization contract.
        // This is not a periodic read path: later terminal work is handled by dirty buckets.
        refresh_long_term_stats(
            &state.pool,
            state.config.long_term_stats_hourly_retention_days,
        )
        .await?;
        baseline_cursor = Some(load_long_term_terminal_watermark(&state.pool).await?);
    } else if cursor == 0 && long_term_rollups_exist(&state.pool).await? {
        // Existing rollups are a durable baseline after upgrade. Only the two open calendar
        // buckets need interval baselines before new terminal deltas can be merged exactly.
        baseline_cursor = Some(load_long_term_terminal_watermark(&state.pool).await?);
    }

    let mut repaired = Vec::new();
    let mut event_count = 0usize;
    if let Some(baseline_cursor) = baseline_cursor {
        let baseline_dates = long_term_projection_open_baseline_dates();
        queue_long_term_projection_repairs(&state.pool, &baseline_dates, "interval_baseline")
            .await?;
        let mut rebuilds = Vec::with_capacity(baseline_dates.len());
        for date in &baseline_dates {
            rebuilds.push(build_long_term_projection_date_rebuild(&state.pool, date).await?);
        }
        commit_long_term_projection_date_rebuilds(
            &state.pool,
            &rebuilds,
            Some(baseline_cursor),
            &baseline_dates,
        )
        .await?;
        repaired.extend(baseline_dates);
        cursor = baseline_cursor;
    } else {
        let identities = load_long_term_account_identities(&state.pool).await?;
        let mut events = load_long_term_projection_terminal_rows(&state.pool, cursor).await?;
        for row in &mut events {
            hydrate_long_term_account_identity(row, &identities);
        }
        let events = events
            .iter()
            .map(build_long_term_projection_event)
            .collect::<Vec<_>>();
        let candidate_dates = events
            .iter()
            .flat_map(|event| event.bucket_dates.iter().cloned())
            .collect::<HashSet<_>>();
        let ready_dates =
            load_long_term_projection_ready_dates(&state.pool, &candidate_dates).await?;
        let mut hourly = HashMap::new();
        let mut daily = HashMap::new();
        let mut segments = Vec::new();
        let mut direct_cursor = cursor;
        let mut repair_event = None;
        for event in events {
            if !event.bucket_dates.is_subset(&ready_dates) {
                repair_event = Some(event);
                break;
            }
            direct_cursor = event.row_id;
            event_count = event_count.saturating_add(1);
            merge_long_term_projection_buckets(&mut hourly, event.hourly);
            merge_long_term_projection_buckets(&mut daily, event.daily);
            segments.extend(event.segments);
        }
        if direct_cursor > cursor {
            apply_long_term_projection_incremental(
                state,
                &hourly,
                &daily,
                &segments,
                direct_cursor,
                event_count,
            )
            .await?;
            cursor = direct_cursor;
        }
        if let Some(event) = repair_event {
            let repair_dates = event.bucket_dates.into_iter().collect::<Vec<_>>();
            queue_long_term_projection_repairs(&state.pool, &repair_dates, "interval_baseline")
                .await?;
            let mut rebuilds = Vec::with_capacity(repair_dates.len());
            for date in &repair_dates {
                rebuilds.push(build_long_term_projection_date_rebuild(&state.pool, date).await?);
            }
            commit_long_term_projection_date_rebuilds(
                &state.pool,
                &rebuilds,
                Some(event.row_id),
                &repair_dates,
            )
            .await?;
            repaired.extend(repair_dates);
            cursor = event.row_id;
        }
    }

    if !repaired.is_empty() {
        invalidate_long_term_projection_interval_cache(state).await;
    }

    let dirty_dates = sqlx::query_scalar::<_, String>(
        "SELECT bucket_date FROM long_term_projection_dirty_buckets WHERE next_attempt_at IS NULL OR datetime(next_attempt_at) <= datetime('now') ORDER BY queued_at ASC, bucket_date ASC LIMIT ?1",
    )
    .bind(LONG_TERM_PROJECTION_MAX_BUCKETS_PER_FLUSH)
    .fetch_all(&state.pool)
    .await?;
    let mut deferred_repair_count = 0usize;
    for date in dirty_dates {
        if repaired.contains(&date) {
            continue;
        }
        let rebuild = match build_long_term_projection_date_rebuild(&state.pool, &date).await {
            Ok(rebuild) => rebuild,
            Err(error) => {
                defer_long_term_projection_repair(&state.pool, &date).await?;
                deferred_repair_count = deferred_repair_count.saturating_add(1);
                warn!(
                    error = %error,
                    projection = "long_term",
                    repair_scope = %date,
                    retry_after_ms = 300_000_u64,
                    "long-term projection repair deferred after an unavailable source"
                );
                continue;
            }
        };
        commit_long_term_projection_date_rebuilds(
            &state.pool,
            &[rebuild],
            None,
            std::slice::from_ref(&date),
        )
        .await?;
        repaired.push(date);
        invalidate_long_term_projection_interval_cache(state).await;
    }

    let (retention_pruned_hourly_rows, retention_pruned_interval_rows) =
        prune_long_term_projection_hourly_retention(
            &state.pool,
            state.config.long_term_stats_hourly_retention_days,
        )
        .await?;

    let dirty_bucket_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_dirty_buckets")
            .fetch_one(&state.pool)
            .await?
            .max(0) as usize;
    state
        .terminal_projection_hub
        .advance_long_term_cursor(cursor);
    let projection_health = state.terminal_projection_hub.health();
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let mut runtime = state.long_term_projection_runtime.lock().await;
    runtime.state = if dirty_bucket_count == 0 {
        "healthy"
    } else {
        "repairing"
    }
    .to_string();
    runtime.cursor_row_id = cursor;
    runtime.dirty_bucket_count = dirty_bucket_count;
    runtime.pending_event_count = projection_health.pending_event_count;
    runtime.last_flush_elapsed_ms = Some(elapsed_ms);
    runtime.last_flush_at = Some(Instant::now());
    runtime.last_repair_scope = (!repaired.is_empty()).then(|| repaired.join(","));
    runtime.last_defer_reason =
        (deferred_repair_count > 0).then(|| "repair_source_unavailable".to_string());
    runtime.last_error_kind = (deferred_repair_count > 0).then(|| "targeted_repair".to_string());
    let interval_bytes = runtime
        .interval_index
        .iter()
        .map(|(key, union)| {
            key.bucket_key.len()
                + key.dimension.len()
                + key.series_key.len()
                + union.intervals.len() * std::mem::size_of::<(i64, i64)>()
        })
        .sum::<usize>();
    debug!(
        projection = "long_term",
        trigger,
        event_count,
        cursor_lag = projection_health.last_persisted_row_id.saturating_sub(cursor),
        dirty_bucket_count,
        repair_scope = ?runtime.last_repair_scope,
        interval_bytes,
        interval_key_count = runtime.interval_index.len(),
        deferred_repair_count,
        retention_pruned_hourly_rows,
        retention_pruned_interval_rows,
        flush_outcome = "accepted",
        elapsed_ms,
        "long-term projection flush completed"
    );
    Ok(())
}

async fn long_term_rollups_exist(pool: &Pool<Sqlite>) -> Result<bool> {
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM long_term_usage_daily LIMIT 1)")
            .fetch_one(pool)
            .await?
            != 0,
    )
}

fn long_term_projection_hourly_retention_start_date(retention_days: u64) -> NaiveDate {
    Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(retention_days.max(366) as i64 - 1)
}

async fn prune_long_term_projection_hourly_retention(
    pool: &Pool<Sqlite>,
    retention_days: u64,
) -> Result<(u64, u64)> {
    let retention_start_date = long_term_projection_hourly_retention_start_date(retention_days);
    let retention_start_epoch = retention_start_date
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Shanghai.from_local_datetime(&value).single())
        .map(|value| value.timestamp())
        .context("invalid long-term projection hourly retention start")?;
    let mut tx = pool.begin().await?;
    let pruned_hourly_rows =
        sqlx::query("DELETE FROM long_term_usage_hourly WHERE bucket_start_epoch < ?1")
            .bind(retention_start_epoch)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    let pruned_interval_rows = sqlx::query(
        "DELETE FROM long_term_projection_intervals WHERE bucket_kind = 'hourly' AND bucket_date < ?1",
    )
    .bind(retention_start_date.to_string())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok((pruned_hourly_rows, pruned_interval_rows))
}

async fn load_long_term_terminal_watermark(pool: &Pool<Sqlite>) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(id), 0) FROM codex_invocations WHERE LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')",
    )
    .fetch_one(pool)
    .await?)
}

async fn load_long_term_projection_cursor(pool: &Pool<Sqlite>) -> Result<i64> {
    sqlx::query(
        "INSERT OR IGNORE INTO long_term_projection_state (consumer, cursor_row_id) VALUES (?1, 0)",
    )
    .bind(LONG_TERM_PROJECTION_CONSUMER)
    .execute(pool)
    .await?;
    Ok(sqlx::query_as::<_, LongTermProjectionCursorRow>(
        "SELECT cursor_row_id FROM long_term_projection_state WHERE consumer = ?1",
    )
    .bind(LONG_TERM_PROJECTION_CONSUMER)
    .fetch_one(pool)
    .await?
    .cursor_row_id)
}

#[derive(Debug)]
struct LongTermProjectionDateRebuild {
    bucket_date: String,
    start_epoch: i64,
    end_epoch: i64,
    hourly: HashMap<(i64, String, String), LongTermBucket>,
    daily: HashMap<(String, String, String), LongTermBucket>,
    interval_segments: Vec<LongTermProjectionIntervalSegment>,
}

fn long_term_projection_row_affects_date(row: &LongTermInvocationRow, bucket_date: &str) -> bool {
    let mut hourly = HashMap::new();
    let mut daily = HashMap::new();
    let mut statistics_start = None;
    accumulate_long_term_invocation(row, &mut hourly, &mut daily, &mut statistics_start);
    daily.keys().any(|(date, _, _)| date == bucket_date)
}

async fn load_long_term_projection_rows_for_date(
    pool: &Pool<Sqlite>,
    date: NaiveDate,
    start: chrono::DateTime<chrono_tz::Tz>,
    end: chrono::DateTime<chrono_tz::Tz>,
) -> Result<Vec<LongTermInvocationRow>> {
    let has_attempt_table = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'pool_upstream_request_attempts')",
    )
    .fetch_one(pool)
    .await?
        != 0;
    let upstream_account_sql = if has_attempt_table {
        "COALESCE(CASE WHEN json_valid(inv.payload) THEN CAST(json_extract(inv.payload, '$.upstreamAccountId') AS INTEGER) END, (SELECT attempt.upstream_account_id FROM pool_upstream_request_attempts attempt WHERE attempt.invoke_id = inv.invoke_id AND attempt.occurred_at = inv.occurred_at AND attempt.upstream_account_id IS NOT NULL ORDER BY attempt.attempt_index DESC, attempt.id DESC LIMIT 1))"
    } else {
        "CASE WHEN json_valid(inv.payload) THEN CAST(json_extract(inv.payload, '$.upstreamAccountId') AS INTEGER) END"
    };
    let query = format!(
        r#"
        SELECT inv.id, inv.invoke_id, inv.occurred_at, inv.status, inv.model,
          CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.requestModel') AS TEXT)), '') END AS request_model,
          CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.responseModel') AS TEXT)), '') END AS response_model,
          CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.reasoningEffort') AS TEXT)), '') END AS reasoning_effort,
          {upstream_account_sql} AS upstream_account_id,
          NULL AS upstream_account_kind, NULL AS upstream_account_name,
          inv.total_tokens, inv.output_tokens, inv.cost, inv.t_total_ms,
          inv.t_req_read_ms, inv.t_req_parse_ms, inv.t_upstream_connect_ms,
          inv.t_upstream_ttfb_ms, inv.t_upstream_stream_ms, inv.error_message
        FROM codex_invocations inv
        WHERE LOWER(TRIM(COALESCE(inv.status, ''))) NOT IN ('running', 'pending')
          AND inv.occurred_at < ?2
          AND (
            inv.occurred_at >= ?1
            OR (
              inv.t_total_ms IS NOT NULL
              AND inv.t_total_ms > 0
              AND julianday(inv.occurred_at) + inv.t_total_ms / 86400000.0 >= julianday(?1)
            )
          )
        ORDER BY inv.occurred_at ASC, inv.id ASC
        "#,
    );
    let mut rows = sqlx::query_as::<_, LongTermInvocationRow>(&query)
        .bind(format_naive(start.naive_local()))
        .bind(format_naive(end.naive_local()))
        .fetch_all(pool)
        .await?;
    let mut row_positions = rows
        .iter()
        .enumerate()
        .map(|(index, row)| (row.id, index))
        .collect::<HashMap<_, _>>();
    let attempt_start = date.pred_opt().unwrap_or(date);
    let (attempt_accounts, _) =
        load_long_term_archive_attempt_accounts(pool, Some((attempt_start, date))).await?;
    let archive_paths = match load_completed_invocation_archive_paths(pool).await {
        Ok(paths) => paths,
        Err(error) if error.to_string().contains("no such table") => Vec::new(),
        Err(error) => return Err(error),
    };
    for archive_path in archive_paths {
        let overlaps = match (
            archive_path
                .coverage_start_at()
                .and_then(long_term_archive_end_date),
            archive_path
                .coverage_end_at()
                .and_then(long_term_archive_end_date),
        ) {
            (Some(path_start), Some(path_end)) => path_start <= date && path_end >= attempt_start,
            // Older manifests without coverage must be inspected rather than treated as absent.
            _ => true,
        };
        if !overlaps {
            continue;
        }
        let Some((archive_pool, cleanup)) = open_invocation_archive_batch_pool(
            &archive_path,
            "long-term-projection-targeted-repair",
        )
        .await?
        else {
            anyhow::bail!(
                "long-term projection source coverage incomplete for {date}: archive {} is unavailable",
                archive_path.file_path()
            );
        };
        let archive_query = long_term_archive_invocation_query_for_range(&archive_pool).await?;
        let archive_rows = sqlx::query_as::<_, LongTermInvocationRow>(&archive_query)
            .bind(format_naive(end.naive_local()))
            .bind(format_naive(start.naive_local()))
            .fetch_all(&archive_pool)
            .await;
        archive_pool.close().await;
        drop(cleanup);
        for mut row in archive_rows? {
            if row.upstream_account_id.is_none()
                && let Some(invoke_id) = row.invoke_id.as_ref()
                && let Some(account_id) =
                    attempt_accounts.get(&(invoke_id.clone(), row.occurred_at.clone()))
            {
                row.upstream_account_id = Some(*account_id);
            }
            if long_term_projection_row_affects_date(&row, &date.to_string()) {
                if let Some(index) = row_positions.get(&row.id).copied() {
                    // Retention can leave a pruned live row with the same id as the archive row.
                    // Preserve the archive's richer fields while using retained columns as fallback.
                    merge_long_term_invocation_row(&mut row, &rows[index]);
                    rows[index] = row;
                } else {
                    row_positions.insert(row.id, rows.len());
                    rows.push(row);
                }
            }
        }
    }
    Ok(rows)
}

async fn build_long_term_projection_date_rebuild(
    pool: &Pool<Sqlite>,
    bucket_date: &str,
) -> Result<LongTermProjectionDateRebuild> {
    let date = NaiveDate::parse_from_str(bucket_date, "%Y-%m-%d")?;
    let start = date
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Shanghai.from_local_datetime(&value).single())
        .context("invalid long-term projection day start")?;
    let end = date
        .succ_opt()
        .and_then(|next| next.and_hms_opt(0, 0, 0))
        .and_then(|value| Shanghai.from_local_datetime(&value).single())
        .context("invalid long-term projection day end")?;
    let mut rows = load_long_term_projection_rows_for_date(pool, date, start, end).await?;
    let identities = load_long_term_account_identities(pool).await?;
    let mut hourly = HashMap::new();
    let mut daily = HashMap::new();
    let mut statistics_start = None;
    let mut interval_segments = Vec::new();
    for row in &mut rows {
        hydrate_long_term_account_identity(row, &identities);
        let mut row_hourly = HashMap::new();
        let mut row_daily = HashMap::new();
        accumulate_long_term_invocation(
            row,
            &mut row_hourly,
            &mut row_daily,
            &mut statistics_start,
        );
        interval_segments.extend(collect_long_term_projection_interval_segments(
            &row_hourly,
            &row_daily,
            row.id,
        ));
        merge_long_term_projection_buckets(&mut hourly, row_hourly);
        merge_long_term_projection_buckets(&mut daily, row_daily);
    }
    hourly.retain(|(hour_epoch, _, _), _| {
        Shanghai
            .timestamp_opt(*hour_epoch, 0)
            .single()
            .is_some_and(|timestamp| timestamp.date_naive() == date)
    });
    daily.retain(|(date_key, _, _), _| date_key == bucket_date);
    interval_segments.retain(|segment| segment.bucket_date == bucket_date);
    Ok(LongTermProjectionDateRebuild {
        bucket_date: bucket_date.to_string(),
        start_epoch: start.timestamp(),
        end_epoch: end.timestamp(),
        hourly,
        daily,
        interval_segments,
    })
}

async fn commit_long_term_projection_date_rebuilds(
    pool: &Pool<Sqlite>,
    rebuilds: &[LongTermProjectionDateRebuild],
    next_cursor: Option<i64>,
    clear_dirty_dates: &[String],
) -> Result<()> {
    let mut transaction = pool.begin().await?;
    for rebuild in rebuilds {
        sqlx::query("DELETE FROM long_term_usage_daily WHERE stats_date = ?1")
            .bind(&rebuild.bucket_date)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM long_term_usage_hourly WHERE bucket_start_epoch >= ?1 AND bucket_start_epoch < ?2")
            .bind(rebuild.start_epoch)
            .bind(rebuild.end_epoch)
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            "DELETE FROM long_term_projection_intervals WHERE (bucket_kind = 'daily' AND bucket_key = ?1) OR (bucket_kind = 'hourly' AND CAST(bucket_key AS INTEGER) >= ?2 AND CAST(bucket_key AS INTEGER) < ?3)",
        )
        .bind(&rebuild.bucket_date)
        .bind(rebuild.start_epoch)
        .bind(rebuild.end_epoch)
        .execute(&mut *transaction)
        .await?;
        for bucket in rebuild.hourly.values() {
            insert_long_term_hourly(&mut transaction, bucket).await?;
        }
        for bucket in rebuild.daily.values() {
            insert_long_term_daily(&mut transaction, bucket).await?;
        }
        let mut interval_index = HashMap::new();
        insert_long_term_projection_interval_segments(
            &mut transaction,
            &rebuild.interval_segments,
            &mut interval_index,
        )
        .await?;
        sqlx::query(
            "INSERT INTO long_term_projection_bucket_state (bucket_date, interval_baseline_ready) VALUES (?1, 1) ON CONFLICT(bucket_date) DO UPDATE SET interval_baseline_ready = 1, updated_at = datetime('now')",
        )
        .bind(&rebuild.bucket_date)
        .execute(&mut *transaction)
        .await?;
    }
    if let Some(cursor) = next_cursor {
        sqlx::query(
            "INSERT INTO long_term_projection_state (consumer, cursor_row_id, last_flush_at, last_error) VALUES (?1, ?2, datetime('now'), NULL) ON CONFLICT(consumer) DO UPDATE SET cursor_row_id = MAX(long_term_projection_state.cursor_row_id, excluded.cursor_row_id), last_flush_at = excluded.last_flush_at, last_error = NULL, updated_at = datetime('now')",
        )
        .bind(LONG_TERM_PROJECTION_CONSUMER)
        .bind(cursor)
        .execute(&mut *transaction)
        .await?;
    }
    for date in clear_dirty_dates {
        sqlx::query("DELETE FROM long_term_projection_dirty_buckets WHERE bucket_date = ?1")
            .bind(date)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    Ok(())
}

async fn rebuild_long_term_projection_date(pool: &Pool<Sqlite>, bucket_date: &str) -> Result<()> {
    let rebuild = build_long_term_projection_date_rebuild(pool, bucket_date).await?;
    commit_long_term_projection_date_rebuilds(pool, &[rebuild], None, &[]).await
}

pub(crate) async fn bootstrap_long_term_integrity_source_boundary_for_legacy_rollups(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    // Earlier schemas had no terminal proof or retirement boundary. The existing canonical
    // history may therefore predate every source that this version can verify. Keep it outside
    // the reconstructable window instead of treating source absence as proof of an empty day.
    let today = Utc::now().with_timezone(&Shanghai).date_naive().to_string();
    sqlx::query(
        r#"
        UPDATE long_term_stats_state
        SET integrity_source_start_date = COALESCE(integrity_source_start_date, ?1),
            integrity_source_pending_start_date = NULL,
            updated_at = datetime('now')
        WHERE id = ?2
        "#,
    )
    .bind(today)
    .bind(LONG_TERM_STATE_ID)
    .execute(pool)
    .await
    .context("failed to bootstrap legacy long-term integrity source boundary")?;
    Ok(())
}

fn long_term_day_epoch_bounds(date: NaiveDate) -> Option<(i64, i64)> {
    let start = date
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Shanghai.from_local_datetime(&value).single())?
        .timestamp();
    let end = date
        .succ_opt()?
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Shanghai.from_local_datetime(&value).single())?
        .timestamp();
    Some((start, end))
}

fn long_term_bucket_date(bucket_start_epoch: i64) -> Option<NaiveDate> {
    Shanghai
        .timestamp_opt(bucket_start_epoch, 0)
        .single()
        .map(|value| value.date_naive())
}

fn long_term_reconstructable_start(
    retention_start: NaiveDate,
    statistics_start_date: Option<&str>,
    integrity_source_start_date: Option<&str>,
) -> NaiveDate {
    let persisted_start = statistics_start_date
        .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
        .unwrap_or(retention_start);
    let integrity_source_start = integrity_source_start_date
        .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
        .unwrap_or(retention_start);
    retention_start
        .max(persisted_start)
        .max(integrity_source_start)
}

fn long_term_source_safe_start_after_effective_date(date: NaiveDate) -> NaiveDate {
    date.succ_opt().unwrap_or(date)
}

fn long_term_unreadable_source_start(
    archive_path: &ArchiveBatchPathRow,
    retention_start: NaiveDate,
) -> NaiveDate {
    archive_path
        .coverage_start_at()
        .and_then(long_term_archive_end_date)
        .unwrap_or(retention_start)
}

async fn clear_long_term_invocation_replay_markers_for_unavailable_sources(
    pool: &Pool<Sqlite>,
    file_paths: &[String],
) -> Result<()> {
    let unique_paths = file_paths.iter().collect::<HashSet<_>>();
    for file_path in unique_paths {
        let cleared = sqlx::query(
            "DELETE FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = 'codex_invocations' AND file_path = ?2",
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .bind(file_path)
        .execute(pool)
        .await?
        .rows_affected();
        if cleared > 0 {
            warn!(
                file_path = %file_path,
                "cleared long-term archive replay marker after source reconciliation could not read the archive"
            );
        }
    }
    Ok(())
}

pub(crate) async fn advance_long_term_integrity_source_start_tx(
    tx: &mut SqliteConnection,
    retiring_archive_batch_id: i64,
    source_safe_start: NaiveDate,
) -> Result<()> {
    let pending_source_start = sqlx::query_scalar::<_, Option<String>>(
        "SELECT integrity_source_pending_start_date FROM long_term_stats_state WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .fetch_optional(&mut *tx)
    .await?
    .flatten()
    .map(|value| {
        NaiveDate::parse_from_str(&value, "%Y-%m-%d").map_err(|error| {
            anyhow!("pending long-term integrity source boundary is invalid ({value}): {error}")
        })
    })
    .transpose()?;
    let candidate = pending_source_start
        .map(|pending| pending.max(source_safe_start))
        .unwrap_or(source_safe_start);
    let candidate_start = candidate.to_string();
    let retained_source_blocks_boundary = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM archive_batches
            WHERE id <> ?1
              AND dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
              AND status = 'completed'
              AND (coverage_start_at IS NULL OR coverage_start_at < ?2)
        )
        "#,
    )
    .bind(retiring_archive_batch_id)
    .bind(&candidate_start)
    .fetch_one(&mut *tx)
    .await?
        != 0;
    if retained_source_blocks_boundary {
        sqlx::query(
            "UPDATE long_term_stats_state SET integrity_source_pending_start_date = ?1, updated_at = datetime('now') WHERE id = ?2",
        )
        .bind(candidate_start)
        .bind(LONG_TERM_STATE_ID)
        .execute(&mut *tx)
        .await?;
        return Ok(());
    }
    sqlx::query(
        r#"
        UPDATE long_term_stats_state
        SET integrity_source_start_date = CASE
            WHEN integrity_source_start_date IS NULL
              OR integrity_source_start_date < ?1 THEN ?1
            ELSE integrity_source_start_date
        END,
        integrity_source_pending_start_date = NULL,
        updated_at = datetime('now')
        WHERE id = ?2
        "#,
    )
    .bind(candidate_start)
    .bind(LONG_TERM_STATE_ID)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(crate) async fn long_term_integrity_source_safe_start_for_archive_cleanup(
    pool: &Pool<Sqlite>,
    dataset: &str,
    file_path: &str,
    coverage_end_at: Option<&str>,
) -> Result<Option<NaiveDate>> {
    match dataset {
        HOURLY_ROLLUP_DATASET_INVOCATIONS => {
            long_term_invocation_archive_safe_start(file_path).await
        }
        "pool_upstream_request_attempts" => {
            long_term_attempt_archive_safe_start(pool, file_path, coverage_end_at).await
        }
        _ => Ok(None),
    }
}

async fn long_term_invocation_archive_safe_start(file_path: &str) -> Result<Option<NaiveDate>> {
    let rows = load_long_term_source_timing_rows_from_archive(
        file_path,
        "long-term-stats-cleanup-invocation-boundary",
    )
    .await?;
    if rows.is_empty() {
        return Ok(None);
    }
    let latest_effective_date = rows
        .iter()
        .map(|row| {
            long_term_source_effective_date(&row.occurred_at, row.t_total_ms).ok_or_else(|| {
                anyhow!(
                    "invocation archive has an unparseable timestamp for source-boundary verification: {}",
                    row.occurred_at
                )
            })
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .max()
        .expect("non-empty invocation archive produces one effective date per source row");
    Ok(Some(long_term_source_safe_start_after_effective_date(
        latest_effective_date,
    )))
}

async fn long_term_attempt_archive_safe_start(
    pool: &Pool<Sqlite>,
    file_path: &str,
    _coverage_end_at: Option<&str>,
) -> Result<Option<NaiveDate>> {
    let Some((archive_pool, cleanup)) = open_pool_upstream_request_attempt_archive_batch_pool(
        &ArchiveBatchPathRow::from_file_path(file_path.to_string()),
        "long-term-stats-cleanup-attempt-boundary",
    )
    .await?
    else {
        bail!("attempt archive is unavailable for long-term source-boundary verification");
    };
    let rows = sqlx::query_as::<_, LongTermArchiveAttemptRow>(
        r#"
        SELECT invoke_id, occurred_at, upstream_account_id
        FROM pool_upstream_request_attempts
        WHERE upstream_account_id IS NOT NULL
        ORDER BY id ASC
        "#,
    )
    .fetch_all(&archive_pool)
    .await;
    archive_pool.close().await;
    drop(cleanup);
    let pairs = rows?
        .into_iter()
        .filter_map(|row| {
            row.upstream_account_id
                .map(|_| (row.invoke_id, row.occurred_at))
        })
        .collect::<HashSet<_>>();
    if pairs.is_empty() {
        return Ok(None);
    }

    let (latest_effective_date, unmatched_pairs) =
        long_term_match_attempt_pairs_to_invocation_sources(pool, pairs).await?;
    if !unmatched_pairs.is_empty() {
        bail!(
            "{} attempt archive account mapping(s) have no readable invocation source",
            unmatched_pairs.len()
        );
    }
    let latest_effective_date = latest_effective_date.ok_or_else(|| {
        anyhow!("attempt archive account mappings have no parseable invocation timestamps")
    })?;
    Ok(Some(long_term_source_safe_start_after_effective_date(
        latest_effective_date,
    )))
}

async fn load_long_term_source_timing_rows_from_archive(
    file_path: &str,
    read_surface: &'static str,
) -> Result<Vec<LongTermSourceTimingRow>> {
    let Some((archive_pool, cleanup)) = open_invocation_archive_batch_pool(
        &ArchiveBatchPathRow::from_file_path(file_path.to_string()),
        read_surface,
    )
    .await?
    else {
        bail!("invocation archive is unavailable for long-term source-boundary verification");
    };
    let archive_columns = load_archive_table_columns(&archive_pool, "codex_invocations").await?;
    if !archive_columns.contains("occurred_at") {
        archive_pool.close().await;
        drop(cleanup);
        bail!("invocation archive has no occurred_at column for source-boundary verification");
    }
    let sql = long_term_source_timing_archive_query(&archive_columns);
    let rows = sqlx::query_as::<_, LongTermSourceTimingRow>(&sql)
        .fetch_all(&archive_pool)
        .await;
    archive_pool.close().await;
    drop(cleanup);
    Ok(rows?)
}

async fn long_term_match_attempt_pairs_to_invocation_sources(
    pool: &Pool<Sqlite>,
    mut unmatched_pairs: HashSet<(String, String)>,
) -> Result<(Option<NaiveDate>, HashSet<(String, String)>)> {
    let mut latest_effective_date = None;
    let live_rows =
        load_long_term_source_timing_rows_for_pairs(pool, &unmatched_pairs, "t_total_ms").await?;
    record_long_term_matched_attempt_source_rows(
        &mut unmatched_pairs,
        &mut latest_effective_date,
        live_rows,
    )?;

    if unmatched_pairs.is_empty() {
        return Ok((latest_effective_date, unmatched_pairs));
    }
    let archive_paths = load_completed_invocation_archive_paths(pool).await?;
    for archive_path in archive_paths {
        if unmatched_pairs.is_empty() {
            break;
        }
        if !long_term_archive_may_contain_attempt_pairs(&archive_path, &unmatched_pairs) {
            continue;
        }
        let Some((archive_pool, cleanup)) = open_invocation_archive_batch_pool(
            &archive_path,
            "long-term-stats-cleanup-attempt-invocation-match",
        )
        .await?
        else {
            bail!("invocation archive is unavailable while verifying attempt account mappings");
        };
        let archive_columns =
            load_archive_table_columns(&archive_pool, "codex_invocations").await?;
        if !archive_columns.contains("invoke_id") || !archive_columns.contains("occurred_at") {
            archive_pool.close().await;
            drop(cleanup);
            bail!("invocation archive lacks keys required to verify attempt account mappings");
        }
        let t_total_ms_expression = long_term_legacy_column_expr(&archive_columns, "t_total_ms");
        let rows = load_long_term_source_timing_rows_for_pairs(
            &archive_pool,
            &unmatched_pairs,
            &t_total_ms_expression,
        )
        .await;
        archive_pool.close().await;
        drop(cleanup);
        record_long_term_matched_attempt_source_rows(
            &mut unmatched_pairs,
            &mut latest_effective_date,
            rows?,
        )?;
    }

    Ok((latest_effective_date, unmatched_pairs))
}

async fn load_long_term_source_timing_rows_for_pairs(
    pool: &Pool<Sqlite>,
    pairs: &HashSet<(String, String)>,
    t_total_ms_expression: &str,
) -> Result<Vec<LongTermSourceTimingRow>> {
    const LONG_TERM_ATTEMPT_SOURCE_QUERY_BATCH_SIZE: usize = 400;

    let pairs = pairs.iter().collect::<Vec<_>>();
    let mut rows = Vec::new();
    for batch in pairs.chunks(LONG_TERM_ATTEMPT_SOURCE_QUERY_BATCH_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new("SELECT invoke_id, occurred_at, ");
        query.push(t_total_ms_expression);
        query.push(" AS t_total_ms FROM codex_invocations WHERE ");
        for (index, (invoke_id, occurred_at)) in batch.iter().enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("(invoke_id = ")
                .push_bind((*invoke_id).clone())
                .push(" AND occurred_at = ")
                .push_bind((*occurred_at).clone())
                .push(")");
        }
        rows.extend(
            query
                .build_query_as::<LongTermSourceTimingRow>()
                .fetch_all(pool)
                .await?,
        );
    }
    Ok(rows)
}

fn long_term_source_timing_archive_query(columns: &HashSet<String>) -> String {
    format!(
        "SELECT {} AS invoke_id, occurred_at, {} AS t_total_ms FROM codex_invocations",
        long_term_legacy_column_expr(columns, "invoke_id"),
        long_term_legacy_column_expr(columns, "t_total_ms"),
    )
}

fn long_term_archive_may_contain_attempt_pairs(
    archive_path: &ArchiveBatchPathRow,
    pairs: &HashSet<(String, String)>,
) -> bool {
    let Some(start) = archive_path
        .coverage_start_at()
        .and_then(long_term_archive_end_date)
    else {
        return true;
    };
    let end = archive_path
        .coverage_end_at()
        .and_then(long_term_archive_end_date)
        .unwrap_or(start);
    pairs.iter().any(|(_, occurred_at)| {
        parse_long_term_timestamp_ms(occurred_at)
            .and_then(|timestamp| Shanghai.timestamp_millis_opt(timestamp).single())
            .map(|timestamp| {
                let date = timestamp.date_naive();
                date >= start && date <= end
            })
            .unwrap_or(true)
    })
}

fn record_long_term_matched_attempt_source_rows(
    unmatched_pairs: &mut HashSet<(String, String)>,
    latest_effective_date: &mut Option<NaiveDate>,
    rows: Vec<LongTermSourceTimingRow>,
) -> Result<()> {
    let requested_pairs = unmatched_pairs.clone();
    for row in rows {
        let Some(invoke_id) = row.invoke_id else {
            continue;
        };
        let pair = (invoke_id, row.occurred_at.clone());
        if !requested_pairs.contains(&pair) {
            continue;
        }
        let date = long_term_source_effective_date(&row.occurred_at, row.t_total_ms).ok_or_else(
            || {
                anyhow!(
                    "matched attempt invocation source has an unparseable timestamp for source-boundary verification: {}",
                    row.occurred_at
                )
            },
        )?;
        unmatched_pairs.remove(&pair);
        *latest_effective_date =
            Some(latest_effective_date.map_or(date, |current| current.max(date)));
    }
    Ok(())
}

fn long_term_source_effective_date(
    occurred_at: &str,
    t_total_ms: Option<f64>,
) -> Option<NaiveDate> {
    let start_ms = parse_long_term_timestamp_ms(occurred_at)?;
    let duration_ms = t_total_ms
        .filter(|value| value.is_finite() && *value > 0.0)
        .map(|value| value.round().clamp(0.0, i64::MAX as f64) as i64)
        .unwrap_or_default();
    let end_ms = start_ms.saturating_add(duration_ms);
    Shanghai
        .timestamp_millis_opt(end_ms)
        .single()
        .map(|timestamp| timestamp.date_naive())
}

fn long_term_integrity_totals_match(
    expected: LongTermIntegrityTotals,
    observed: LongTermIntegrityTotals,
) -> bool {
    expected.calls == observed.calls
        && expected.token_total == observed.token_total
        && (expected.cost_total - observed.cost_total).abs()
            <= 1e-6_f64.max(expected.cost_total.abs() * 1e-9)
}

fn long_term_integrity_totals_are_empty(totals: LongTermIntegrityTotals) -> bool {
    totals.calls == 0 && totals.token_total == 0 && totals.cost_total.abs() <= 1e-12
}

fn long_term_integrity_mismatch(
    date: NaiveDate,
    expected_daily: LongTermIntegrityTotals,
    expected_hourly: &HashMap<i64, LongTermIntegrityTotals>,
    observed_daily: LongTermIntegrityTotals,
    observed_hourly: &HashMap<i64, LongTermIntegrityTotals>,
) -> Option<LongTermIntegrityMismatch> {
    if !long_term_integrity_totals_match(expected_daily, observed_daily) {
        return Some(LongTermIntegrityMismatch {
            date,
            expected: expected_daily,
            observed: observed_daily,
            reason: format!(
                "daily overall differs: expected calls={}, tokens={}, cost={:.9}; observed calls={}, tokens={}, cost={:.9}",
                expected_daily.calls,
                expected_daily.token_total,
                expected_daily.cost_total,
                observed_daily.calls,
                observed_daily.token_total,
                observed_daily.cost_total,
            ),
        });
    }
    for (bucket_start_epoch, expected) in expected_hourly {
        let observed = observed_hourly
            .get(bucket_start_epoch)
            .copied()
            .unwrap_or_default();
        if !long_term_integrity_totals_match(*expected, observed) {
            return Some(LongTermIntegrityMismatch {
                date,
                expected: *expected,
                observed,
                reason: format!(
                    "hourly overall differs at {}: expected calls={}, tokens={}, cost={:.9}; observed calls={}, tokens={}, cost={:.9}",
                    bucket_start_epoch,
                    expected.calls,
                    expected.token_total,
                    expected.cost_total,
                    observed.calls,
                    observed.token_total,
                    observed.cost_total,
                ),
            });
        }
    }
    for (bucket_start_epoch, observed) in observed_hourly {
        if !expected_hourly.contains_key(bucket_start_epoch)
            && !long_term_integrity_totals_are_empty(*observed)
        {
            return Some(LongTermIntegrityMismatch {
                date,
                expected: LongTermIntegrityTotals::default(),
                observed: *observed,
                reason: format!(
                    "hourly overall has an unexpected non-empty bucket at {bucket_start_epoch}"
                ),
            });
        }
    }
    None
}

async fn long_term_integrity_oracle_available(pool: &Pool<Sqlite>) -> Result<bool> {
    let columns = load_sqlite_table_columns(pool, "invocation_rollup_hourly").await?;
    Ok([
        "terminal_count",
        "terminal_tokens",
        "terminal_cost",
        "terminal_proof_complete",
    ]
    .into_iter()
    .all(|column| columns.contains(column)))
}

async fn load_long_term_integrity_oracle(
    pool: &Pool<Sqlite>,
    date: NaiveDate,
) -> Result<Option<LongTermIntegrityOracle>> {
    if !long_term_integrity_oracle_available(pool).await? {
        return Ok(None);
    }
    let Some((start_epoch, end_epoch)) = long_term_day_epoch_bounds(date) else {
        return Ok(None);
    };
    let has_untrusted_hour = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM invocation_rollup_hourly
            WHERE bucket_start_epoch >= ?1
              AND bucket_start_epoch < ?2
              AND terminal_proof_complete <> 1
        )
        "#,
    )
    .bind(start_epoch)
    .bind(end_epoch)
    .fetch_one(pool)
    .await?
        != 0;
    if has_untrusted_hour {
        return Ok(None);
    }
    let rows = sqlx::query_as::<_, LongTermIntegrityHourRow>(
        r#"
        SELECT bucket_start_epoch,
               COALESCE(SUM(terminal_count), 0) AS calls,
               COALESCE(SUM(terminal_tokens), 0) AS token_total,
               COALESCE(SUM(terminal_cost), 0.0) AS cost_total
        FROM invocation_rollup_hourly
        WHERE bucket_start_epoch >= ?1 AND bucket_start_epoch < ?2
        GROUP BY bucket_start_epoch
        "#,
    )
    .bind(start_epoch)
    .bind(end_epoch)
    .fetch_all(pool)
    .await?;
    let mut daily = LongTermIntegrityTotals::default();
    let mut hourly = HashMap::new();
    for row in rows {
        let totals = LongTermIntegrityTotals {
            calls: row.calls,
            token_total: row.token_total,
            cost_total: row.cost_total,
        };
        daily.calls += totals.calls;
        daily.token_total += totals.token_total;
        daily.cost_total += totals.cost_total;
        hourly.insert(row.bucket_start_epoch, totals);
    }
    Ok(Some(LongTermIntegrityOracle {
        date,
        daily,
        hourly,
    }))
}

fn long_term_candidate_integrity(
    date: NaiveDate,
    hourly: &HashMap<(i64, String, String), LongTermBucket>,
    daily: &HashMap<(String, String, String), LongTermBucket>,
) -> (
    LongTermIntegrityTotals,
    HashMap<i64, LongTermIntegrityTotals>,
) {
    let date_string = date.to_string();
    let daily_totals = daily
        .iter()
        .filter(|((bucket_date, dimension, _), _)| {
            bucket_date == &date_string && dimension == "overall"
        })
        .fold(
            LongTermIntegrityTotals::default(),
            |mut totals, (_, bucket)| {
                totals.calls += bucket.accumulator.calls;
                totals.token_total += bucket.accumulator.token_total;
                totals.cost_total += bucket.accumulator.cost_total;
                totals
            },
        );
    let mut hourly_totals = HashMap::new();
    for ((bucket_start_epoch, dimension, _), bucket) in hourly {
        if dimension != "overall" || long_term_bucket_date(*bucket_start_epoch) != Some(date) {
            continue;
        }
        let totals = hourly_totals
            .entry(*bucket_start_epoch)
            .or_insert_with(LongTermIntegrityTotals::default);
        totals.calls += bucket.accumulator.calls;
        totals.token_total += bucket.accumulator.token_total;
        totals.cost_total += bucket.accumulator.cost_total;
    }
    (daily_totals, hourly_totals)
}

fn remove_long_term_candidate_dates(
    hourly: &mut HashMap<(i64, String, String), LongTermBucket>,
    daily: &mut HashMap<(String, String, String), LongTermBucket>,
    dates: &HashSet<NaiveDate>,
) {
    if dates.is_empty() {
        return;
    }
    hourly.retain(|(bucket_start, _, _), _| {
        Shanghai
            .timestamp_opt(*bucket_start, 0)
            .single()
            .map(|value| !dates.contains(&value.date_naive()))
            .unwrap_or(true)
    });
    daily.retain(|(date, _, _), _| {
        NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map(|value| !dates.contains(&value))
            .unwrap_or(true)
    });
}

async fn enqueue_long_term_integrity_mismatch(
    pool: &Pool<Sqlite>,
    mismatch: &LongTermIntegrityMismatch,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO long_term_stats_repair_queue (
            stats_date,
            expected_calls,
            expected_token_total,
            expected_cost_total,
            observed_calls,
            observed_token_total,
            observed_cost_total,
            last_error
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        ON CONFLICT(stats_date) DO UPDATE SET
            expected_calls = excluded.expected_calls,
            expected_token_total = excluded.expected_token_total,
            expected_cost_total = excluded.expected_cost_total,
            observed_calls = excluded.observed_calls,
            observed_token_total = excluded.observed_token_total,
            observed_cost_total = excluded.observed_cost_total,
            last_error = excluded.last_error,
            updated_at = datetime('now')
        "#,
    )
    .bind(mismatch.date.to_string())
    .bind(mismatch.expected.calls)
    .bind(mismatch.expected.token_total)
    .bind(mismatch.expected.cost_total)
    .bind(mismatch.observed.calls)
    .bind(mismatch.observed.token_total)
    .bind(mismatch.observed.cost_total)
    .bind(&mismatch.reason)
    .execute(pool)
    .await?;
    Ok(())
}

async fn load_long_term_reconciliation_mismatches(
    pool: &Pool<Sqlite>,
    invalidated_bucket_start_epochs: &[i64],
    reconstructable_start: NaiveDate,
) -> Result<Vec<LongTermIntegrityMismatch>> {
    let dates = invalidated_bucket_start_epochs
        .iter()
        .filter_map(|bucket_start_epoch| long_term_bucket_date(*bucket_start_epoch))
        .filter(|date| *date >= reconstructable_start)
        .collect::<BTreeSet<_>>();
    let mut mismatches = Vec::with_capacity(dates.len());
    for date in dates {
        let Some((start_epoch, end_epoch)) = long_term_day_epoch_bounds(date) else {
            continue;
        };
        // The canonical values remain useful as the durable expectation for a queued repair,
        // even after their proof bit is revoked. A later source reconciliation must restore the
        // proof before the repair path is allowed to replace the long-term rows.
        let expected = sqlx::query_as::<_, (i64, i64, f64)>(
            r#"
            SELECT COALESCE(SUM(terminal_count), 0),
                   COALESCE(SUM(terminal_tokens), 0),
                   COALESCE(SUM(terminal_cost), 0.0)
            FROM invocation_rollup_hourly
            WHERE bucket_start_epoch >= ?1 AND bucket_start_epoch < ?2
            "#,
        )
        .bind(start_epoch)
        .bind(end_epoch)
        .fetch_one(pool)
        .await?;
        let observed = sqlx::query_as::<_, (i64, i64, f64)>(
            r#"
            SELECT COALESCE(SUM(calls), 0),
                   COALESCE(SUM(token_total), 0),
                   COALESCE(SUM(cost_total), 0.0)
            FROM long_term_usage_daily
            WHERE stats_date = ?1 AND dimension = 'overall'
            "#,
        )
        .bind(date.to_string())
        .fetch_one(pool)
        .await?;
        mismatches.push(LongTermIntegrityMismatch {
            date,
            expected: LongTermIntegrityTotals {
                calls: expected.0,
                token_total: expected.1,
                cost_total: expected.2,
            },
            observed: LongTermIntegrityTotals {
                calls: observed.0,
                token_total: observed.1,
                cost_total: observed.2,
            },
            reason: "complete invocation source reconciliation disagreed with canonical hourly terminal totals".to_string(),
        });
    }
    Ok(mismatches)
}

async fn next_due_long_term_repair_date(
    pool: &Pool<Sqlite>,
    reconstructable_start: NaiveDate,
) -> Result<Option<NaiveDate>> {
    let date = sqlx::query_scalar::<_, String>(
        r#"
        SELECT stats_date
        FROM long_term_stats_repair_queue
        WHERE stats_date >= ?1
          AND datetime(next_retry_at) <= datetime('now')
        ORDER BY datetime(next_retry_at) ASC, stats_date ASC
        LIMIT 1
        "#,
    )
    .bind(reconstructable_start.to_string())
    .fetch_optional(pool)
    .await?;
    Ok(date.and_then(|value| NaiveDate::parse_from_str(&value, "%Y-%m-%d").ok()))
}

async fn count_reconstructable_long_term_repairs(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    reconstructable_start: NaiveDate,
) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM long_term_stats_repair_queue WHERE stats_date >= ?1",
    )
    .bind(reconstructable_start.to_string())
    .fetch_one(&mut **tx)
    .await?)
}

async fn long_term_has_persisted_integrity_damage(
    pool: &Pool<Sqlite>,
    reconstructable_start: NaiveDate,
) -> Result<bool> {
    let has_pending_repairs = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM long_term_stats_repair_queue WHERE stats_date >= ?1)",
    )
    .bind(reconstructable_start.to_string())
    .fetch_one(pool)
    .await?
        != 0;
    let status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_optional(pool)
            .await?;
    Ok(has_pending_repairs || status.as_deref() == Some(LONG_TERM_STATUS_ERROR))
}

async fn queued_long_term_repair_mismatch(
    pool: &Pool<Sqlite>,
    date: NaiveDate,
    observed: LongTermIntegrityTotals,
    reason: impl Into<String>,
) -> Result<Option<LongTermIntegrityMismatch>> {
    let expected = sqlx::query_as::<_, (i64, i64, f64)>(
        "SELECT expected_calls, expected_token_total, expected_cost_total FROM long_term_stats_repair_queue WHERE stats_date = ?1",
    )
    .bind(date.to_string())
    .fetch_optional(pool)
    .await?;
    Ok(expected.map(
        |(calls, token_total, cost_total)| LongTermIntegrityMismatch {
            date,
            expected: LongTermIntegrityTotals {
                calls,
                token_total,
                cost_total,
            },
            observed,
            reason: reason.into(),
        },
    ))
}

async fn long_term_integrity_audit_due(pool: &Pool<Sqlite>) -> Result<bool> {
    let due = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT CASE
            WHEN last_integrity_audit_at IS NULL THEN 1
            WHEN datetime(last_integrity_audit_at) <= datetime('now', ?1) THEN 1
            ELSE 0
        END
        FROM long_term_stats_state
        WHERE id = ?2
        "#,
    )
    .bind(format!(
        "-{} seconds",
        LONG_TERM_INTEGRITY_AUDIT_INTERVAL_SECS
    ))
    .bind(LONG_TERM_STATE_ID)
    .fetch_optional(pool)
    .await?
    .unwrap_or(1);
    Ok(due != 0)
}

async fn mark_long_term_integrity_audit(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        "UPDATE long_term_stats_state SET last_integrity_audit_at = datetime('now') WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .execute(pool)
    .await?;
    Ok(())
}

async fn audit_long_term_integrity(
    pool: &Pool<Sqlite>,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<Vec<LongTermIntegrityMismatch>> {
    if start_date > end_date || !long_term_integrity_oracle_available(pool).await? {
        return Ok(Vec::new());
    }
    let Some((start_epoch, _)) = long_term_day_epoch_bounds(start_date) else {
        return Ok(Vec::new());
    };
    let Some((_, end_epoch)) = long_term_day_epoch_bounds(end_date) else {
        return Ok(Vec::new());
    };
    let untrusted_dates = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT DISTINCT bucket_start_epoch
        FROM invocation_rollup_hourly
        WHERE bucket_start_epoch >= ?1
          AND bucket_start_epoch < ?2
          AND terminal_proof_complete <> 1
        "#,
    )
    .bind(start_epoch)
    .bind(end_epoch)
    .fetch_all(pool)
    .await?
    .into_iter()
    .filter_map(long_term_bucket_date)
    .collect::<HashSet<_>>();
    let expected_rows = sqlx::query_as::<_, LongTermIntegrityHourRow>(
        r#"
        SELECT bucket_start_epoch,
               COALESCE(SUM(terminal_count), 0) AS calls,
               COALESCE(SUM(terminal_tokens), 0) AS token_total,
               COALESCE(SUM(terminal_cost), 0.0) AS cost_total
        FROM invocation_rollup_hourly
        WHERE bucket_start_epoch >= ?1
          AND bucket_start_epoch < ?2
          AND terminal_proof_complete = 1
        GROUP BY bucket_start_epoch
        "#,
    )
    .bind(start_epoch)
    .bind(end_epoch)
    .fetch_all(pool)
    .await?;
    let materialized_daily_dates = sqlx::query_scalar::<_, String>(
        r#"
        SELECT DISTINCT stats_date
        FROM long_term_usage_daily
        WHERE stats_date >= ?1 AND stats_date <= ?2
        "#,
    )
    .bind(start_date.to_string())
    .bind(end_date.to_string())
    .fetch_all(pool)
    .await?;
    let materialized_hourly_buckets = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT DISTINCT bucket_start_epoch
        FROM long_term_usage_hourly
        WHERE bucket_start_epoch >= ?1 AND bucket_start_epoch < ?2
        "#,
    )
    .bind(start_epoch)
    .bind(end_epoch)
    .fetch_all(pool)
    .await?;
    let materialized_nonempty_daily_dates = sqlx::query_scalar::<_, String>(
        r#"
        SELECT DISTINCT stats_date
        FROM long_term_usage_daily
        WHERE stats_date >= ?1 AND stats_date <= ?2
          AND (calls <> 0 OR token_total <> 0 OR ABS(cost_total) > 1e-12)
        "#,
    )
    .bind(start_date.to_string())
    .bind(end_date.to_string())
    .fetch_all(pool)
    .await?;
    let materialized_nonempty_hourly_buckets = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT DISTINCT bucket_start_epoch
        FROM long_term_usage_hourly
        WHERE bucket_start_epoch >= ?1 AND bucket_start_epoch < ?2
          AND (calls <> 0 OR token_total <> 0 OR ABS(cost_total) > 1e-12)
        "#,
    )
    .bind(start_epoch)
    .bind(end_epoch)
    .fetch_all(pool)
    .await?;
    let actual_hourly_rows = sqlx::query_as::<_, LongTermIntegrityHourRow>(
        r#"
        SELECT bucket_start_epoch,
               COALESCE(SUM(calls), 0) AS calls,
               COALESCE(SUM(token_total), 0) AS token_total,
               COALESCE(SUM(cost_total), 0.0) AS cost_total
        FROM long_term_usage_hourly
        WHERE dimension = 'overall'
          AND bucket_start_epoch >= ?1
          AND bucket_start_epoch < ?2
        GROUP BY bucket_start_epoch
        "#,
    )
    .bind(start_epoch)
    .bind(end_epoch)
    .fetch_all(pool)
    .await?;
    let actual_daily_rows = sqlx::query_as::<_, (String, i64, i64, f64)>(
        r#"
        SELECT stats_date,
               COALESCE(SUM(calls), 0),
               COALESCE(SUM(token_total), 0),
               COALESCE(SUM(cost_total), 0.0)
        FROM long_term_usage_daily
        WHERE dimension = 'overall' AND stats_date >= ?1 AND stats_date <= ?2
        GROUP BY stats_date
        "#,
    )
    .bind(start_date.to_string())
    .bind(end_date.to_string())
    .fetch_all(pool)
    .await?;

    let mut expected_by_date: HashMap<NaiveDate, HashMap<i64, LongTermIntegrityTotals>> =
        HashMap::new();
    for row in expected_rows {
        let Some(date) = long_term_bucket_date(row.bucket_start_epoch) else {
            continue;
        };
        expected_by_date.entry(date).or_default().insert(
            row.bucket_start_epoch,
            LongTermIntegrityTotals {
                calls: row.calls,
                token_total: row.token_total,
                cost_total: row.cost_total,
            },
        );
    }
    let mut actual_hourly_by_date: HashMap<NaiveDate, HashMap<i64, LongTermIntegrityTotals>> =
        HashMap::new();
    for row in actual_hourly_rows {
        let Some(date) = long_term_bucket_date(row.bucket_start_epoch) else {
            continue;
        };
        actual_hourly_by_date.entry(date).or_default().insert(
            row.bucket_start_epoch,
            LongTermIntegrityTotals {
                calls: row.calls,
                token_total: row.token_total,
                cost_total: row.cost_total,
            },
        );
    }
    let actual_daily_by_date = actual_daily_rows
        .into_iter()
        .filter_map(|(date, calls, token_total, cost_total)| {
            NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                .ok()
                .map(|date| {
                    (
                        date,
                        LongTermIntegrityTotals {
                            calls,
                            token_total,
                            cost_total,
                        },
                    )
                })
        })
        .collect::<HashMap<_, _>>();
    let mut materialized_dates = materialized_daily_dates
        .into_iter()
        .filter_map(|date| NaiveDate::parse_from_str(&date, "%Y-%m-%d").ok())
        .collect::<HashSet<_>>();
    materialized_dates.extend(
        materialized_hourly_buckets
            .into_iter()
            .filter_map(long_term_bucket_date),
    );
    let mut materialized_nonempty_dates = materialized_nonempty_daily_dates
        .into_iter()
        .filter_map(|date| NaiveDate::parse_from_str(&date, "%Y-%m-%d").ok())
        .collect::<HashSet<_>>();
    materialized_nonempty_dates.extend(
        materialized_nonempty_hourly_buckets
            .into_iter()
            .filter_map(long_term_bucket_date),
    );

    // Audit both sides of the comparison. A non-empty materialized day with no canonical
    // hourly rows is corrupt, but a zero-total wall-time continuation from a prior-day call is
    // legitimate and cannot be disproved by a start-hour canonical rollup.
    let mut audit_dates = expected_by_date
        .keys()
        .chain(actual_hourly_by_date.keys())
        .chain(actual_daily_by_date.keys())
        .chain(materialized_dates.iter())
        .copied()
        .collect::<Vec<_>>();
    audit_dates.sort_unstable();
    audit_dates.dedup();
    let empty_hourly = HashMap::new();
    let mut mismatches = Vec::new();
    for date in audit_dates {
        if untrusted_dates.contains(&date) {
            continue;
        }
        let expected_hourly = expected_by_date.get(&date).unwrap_or(&empty_hourly);
        let expected_daily = expected_hourly.values().fold(
            LongTermIntegrityTotals::default(),
            |mut totals, value| {
                totals.calls += value.calls;
                totals.token_total += value.token_total;
                totals.cost_total += value.cost_total;
                totals
            },
        );
        if long_term_integrity_totals_are_empty(expected_daily)
            && materialized_dates.contains(&date)
            && materialized_nonempty_dates.contains(&date)
        {
            mismatches.push(LongTermIntegrityMismatch {
                date,
                expected: LongTermIntegrityTotals::default(),
                observed: actual_daily_by_date.get(&date).copied().unwrap_or_default(),
                reason: "canonical terminal totals are empty but materialized rows remain in one or more dimensions".to_string(),
            });
            continue;
        }
        if let Some(mismatch) = long_term_integrity_mismatch(
            date,
            expected_daily,
            expected_hourly,
            actual_daily_by_date.get(&date).copied().unwrap_or_default(),
            actual_hourly_by_date.get(&date).unwrap_or(&empty_hourly),
        ) {
            mismatches.push(mismatch);
        }
    }
    Ok(mismatches)
}

fn long_term_repair_backoff_secs(attempts: i64) -> i64 {
    let index = attempts.saturating_sub(1) as usize;
    LONG_TERM_REPAIR_BACKOFF_SECS[index.min(LONG_TERM_REPAIR_BACKOFF_SECS.len() - 1)]
}

async fn schedule_long_term_repair_retry(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    mismatch: &LongTermIntegrityMismatch,
) -> Result<()> {
    let attempts = sqlx::query_scalar::<_, i64>(
        "SELECT attempts FROM long_term_stats_repair_queue WHERE stats_date = ?1",
    )
    .bind(mismatch.date.to_string())
    .fetch_optional(&mut **tx)
    .await?
    .unwrap_or(0)
        + 1;
    let retry_modifier = format!("+{} seconds", long_term_repair_backoff_secs(attempts));
    sqlx::query(
        r#"
        INSERT INTO long_term_stats_repair_queue (
            stats_date,
            expected_calls,
            expected_token_total,
            expected_cost_total,
            observed_calls,
            observed_token_total,
            observed_cost_total,
            attempts,
            next_retry_at,
            last_error
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now', ?9), ?10)
        ON CONFLICT(stats_date) DO UPDATE SET
            expected_calls = excluded.expected_calls,
            expected_token_total = excluded.expected_token_total,
            expected_cost_total = excluded.expected_cost_total,
            observed_calls = excluded.observed_calls,
            observed_token_total = excluded.observed_token_total,
            observed_cost_total = excluded.observed_cost_total,
            attempts = excluded.attempts,
            next_retry_at = excluded.next_retry_at,
            last_error = excluded.last_error,
            updated_at = datetime('now')
        "#,
    )
    .bind(mismatch.date.to_string())
    .bind(mismatch.expected.calls)
    .bind(mismatch.expected.token_total)
    .bind(mismatch.expected.cost_total)
    .bind(mismatch.observed.calls)
    .bind(mismatch.observed.token_total)
    .bind(mismatch.observed.cost_total)
    .bind(attempts)
    .bind(retry_modifier)
    .bind(&mismatch.reason)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn mark_long_term_stats_backfill_preparing(pool: &Pool<Sqlite>) -> Result<()> {
    // Preserve error across restart so a persisted integrity queue keeps the next refresh on the
    // verified incremental path. Error without durable rows still transitions to running inside
    // refresh_long_term_stats_once.
    sqlx::query(
        "UPDATE long_term_stats_state SET status = ?1, updated_at = datetime('now') WHERE id = ?2 AND status NOT IN (?3, ?4)",
    )
    .bind(LONG_TERM_STATUS_PREPARING)
    .bind(LONG_TERM_STATE_ID)
    .bind(LONG_TERM_STATUS_READY)
    .bind(LONG_TERM_STATUS_ERROR)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) fn spawn_long_term_stats_backfill(
    pool: Pool<Sqlite>,
    retention_days: u64,
    shutdown: CancellationToken,
) {
    tokio::spawn(async move {
        let _ = mark_long_term_stats_backfill_preparing(&pool).await;
        let mut ticker = interval(Duration::from_secs(60));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            if shutdown.is_cancelled() {
                break;
            }
            if let Err(err) = refresh_long_term_stats(&pool, retention_days).await {
                warn!(error = %err, "long-term stats materialization failed");
            }
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = ticker.tick() => {}
            }
        }
    });
}

pub(crate) async fn refresh_long_term_stats(
    pool: &Pool<Sqlite>,
    retention_days: u64,
) -> Result<()> {
    let _guard = LONG_TERM_REFRESH_LOCK.lock().await;
    run_long_term_refresh_with_retry(|| refresh_long_term_stats_once(pool, retention_days)).await
}

async fn run_long_term_refresh_with_retry<T, Operation, OperationFuture>(
    operation: Operation,
) -> Result<T>
where
    Operation: FnMut() -> OperationFuture,
    OperationFuture: Future<Output = Result<T>>,
{
    run_long_term_refresh_with_retry_delays(operation, &LONG_TERM_REFRESH_LOCK_RETRY_DELAYS).await
}

async fn run_long_term_refresh_with_retry_delays<T, Operation, OperationFuture>(
    mut operation: Operation,
    retry_delays: &[Duration],
) -> Result<T>
where
    Operation: FnMut() -> OperationFuture,
    OperationFuture: Future<Output = Result<T>>,
{
    for (attempt, delay) in retry_delays.iter().enumerate() {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(error) if crate::is_sqlite_lock_error(&error) => {
                warn!(
                    attempt = attempt + 1,
                    retry_after_ms = delay.as_millis(),
                    error = %error,
                    "long-term stats refresh hit a SQLite lock; retrying"
                );
                sleep(*delay).await;
            }
            Err(error) => return Err(error),
        }
    }
    operation().await
}

fn long_term_refresh_start_state(
    was_ready: bool,
    has_pending_integrity_repairs: bool,
) -> (&'static str, bool) {
    if has_pending_integrity_repairs {
        (LONG_TERM_STATUS_ERROR, false)
    } else if !was_ready {
        (LONG_TERM_STATUS_RUNNING, true)
    } else {
        (LONG_TERM_STATUS_READY, true)
    }
}

async fn refresh_long_term_stats_once(pool: &Pool<Sqlite>, retention_days: u64) -> Result<()> {
    let refresh_started_at = format_utc_iso(Utc::now());
    let state_snapshot = sqlx::query_as::<_, (Option<String>, Option<String>, Option<String>)>(
        "SELECT status, statistics_start_date, integrity_source_start_date FROM long_term_stats_state WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .fetch_optional(pool)
    .await?;
    let status_allows_incremental_refresh =
        state_snapshot.as_ref().is_some_and(|(status, _, _)| {
            status.as_deref().is_some_and(|status| {
                matches!(
                    status,
                    LONG_TERM_STATUS_READY | LONG_TERM_STATUS_EMPTY | LONG_TERM_STATUS_ERROR
                )
            })
        });
    let has_persisted_daily_rows = if status_allows_incremental_refresh {
        sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM long_term_usage_daily LIMIT 1)")
            .fetch_one(pool)
            .await?
            != 0
    } else {
        false
    };
    let was_ready = state_snapshot.as_ref().is_some_and(|(status, _, _)| {
        status.as_deref().is_some_and(|status| {
            matches!(status, LONG_TERM_STATUS_READY | LONG_TERM_STATUS_EMPTY)
                || (status == LONG_TERM_STATUS_ERROR && has_persisted_daily_rows)
        })
    });
    let today = Utc::now().with_timezone(&Shanghai).date_naive();
    let retention_start = today - ChronoDuration::days(retention_days.max(366) as i64 - 1);
    let reconstructable_start = long_term_reconstructable_start(
        retention_start,
        state_snapshot
            .as_ref()
            .and_then(|(_, statistics_start_date, _)| statistics_start_date.as_deref()),
        state_snapshot
            .as_ref()
            .and_then(|(_, _, integrity_source_start_date)| integrity_source_start_date.as_deref()),
    );
    let has_pending_integrity_repairs = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM long_term_stats_repair_queue WHERE stats_date >= ?1)",
    )
    .bind(reconstructable_start.to_string())
    .fetch_one(pool)
    .await?
        != 0;
    let preserves_prior_error = state_snapshot.as_ref().is_some_and(|(status, _, _)| {
        status
            .as_deref()
            .is_some_and(|status| status == LONG_TERM_STATUS_ERROR)
    });
    let (starting_status, clear_last_error) = long_term_refresh_start_state(
        was_ready,
        has_pending_integrity_repairs || preserves_prior_error,
    );
    if clear_last_error {
        sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, last_error = NULL, updated_at = datetime('now') WHERE id = ?2",
        )
        .bind(starting_status)
        .bind(LONG_TERM_STATE_ID)
        .execute(pool)
        .await?;
    } else {
        // Keep known-bad materialized data hidden throughout a repair attempt. The final
        // replacement transaction is the only path that clears the queue and restores ready.
        sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, updated_at = datetime('now') WHERE id = ?2",
        )
        .bind(starting_status)
        .bind(LONG_TERM_STATE_ID)
        .execute(pool)
        .await?;
    }

    let result =
        refresh_long_term_stats_inner(pool, retention_days, was_ready, &refresh_started_at).await;
    if let Err(err) = &result {
        let source_attribution_is_unavailable = err
            .to_string()
            .contains(LONG_TERM_ATTEMPT_ARCHIVE_UNAVAILABLE_ERROR);
        let persisted_integrity_damage =
            long_term_has_persisted_integrity_damage(pool, reconstructable_start)
                .await
                .unwrap_or(false);
        let _ = sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, last_error = ?2, updated_at = datetime('now') WHERE id = ?3 AND NOT (status = ?4 AND datetime(updated_at) > datetime(?5))",
        )
        .bind(if was_ready
            && !has_pending_integrity_repairs
            && !preserves_prior_error
            && !persisted_integrity_damage
            && !source_attribution_is_unavailable
        {
            LONG_TERM_STATUS_READY
        } else {
            LONG_TERM_STATUS_ERROR
        })
        .bind(err.to_string())
        .bind(LONG_TERM_STATE_ID)
        .bind(LONG_TERM_STATUS_PREPARING)
        .bind(&refresh_started_at)
        .execute(pool)
        .await;
    }
    result
}

async fn refresh_long_term_stats_inner(
    pool: &Pool<Sqlite>,
    retention_days: u64,
    was_ready: bool,
    refresh_started_at: &str,
) -> Result<()> {
    let previous_state = sqlx::query_as::<_, LongTermStateRow>(
        "SELECT status, statistics_start_date, integrity_source_start_date, processed_rows, total_rows, last_error FROM long_term_stats_state WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .fetch_optional(pool)
    .await?;
    let today = Utc::now().with_timezone(&Shanghai).date_naive();
    let live_tail_start = (today - ChronoDuration::days(2))
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Shanghai.from_local_datetime(&value).single())
        .map(|value| db_occurred_at_lower_bound(value.with_timezone(&Utc)));
    let has_attempt_table = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'pool_upstream_request_attempts')",
    )
    .fetch_one(pool)
    .await?
        != 0;
    let live_upstream_account_id_sql = if has_attempt_table {
        "COALESCE(CASE WHEN json_valid(inv.payload) THEN CAST(json_extract(inv.payload, '$.upstreamAccountId') AS INTEGER) END, (SELECT attempt.upstream_account_id FROM pool_upstream_request_attempts attempt WHERE attempt.invoke_id = inv.invoke_id AND attempt.occurred_at = inv.occurred_at AND attempt.upstream_account_id IS NOT NULL ORDER BY attempt.attempt_index DESC, attempt.id DESC LIMIT 1))"
    } else {
        "CASE WHEN json_valid(inv.payload) THEN CAST(json_extract(inv.payload, '$.upstreamAccountId') AS INTEGER) END"
    };
    let legacy_model_keys = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM long_term_usage_daily WHERE dimension = 'model' AND series_key NOT LIKE 'model:v2:%')",
    )
    .fetch_one(pool)
    .await?
        != 0;
    let ready_state = was_ready && !legacy_model_keys;
    let retention_start = today - ChronoDuration::days(retention_days.max(366) as i64 - 1);
    let integrity_audit_due = ready_state && long_term_integrity_audit_due(pool).await?;
    let mut terminal_proof_reconciliation_incomplete =
        previous_state.as_ref().is_some_and(|state| {
            state.status == LONG_TERM_STATUS_ERROR
                && state.last_error.as_deref() == Some(LONG_TERM_TERMINAL_PROOF_UNAVAILABLE_ERROR)
        });
    let mut invalidated_terminal_proof_buckets = Vec::new();
    let mut unavailable_reconciliation_archive_paths = Vec::new();
    // A source-availability failure is retryable as soon as the next refresh runs. Waiting for
    // the hourly audit after a restored archive would leave an otherwise recoverable view in
    // error for up to an hour.
    if !ready_state || integrity_audit_due || terminal_proof_reconciliation_incomplete {
        match backfill_invocation_rollup_hourly_from_sources(pool).await {
            Ok(reconciliation) => {
                terminal_proof_reconciliation_incomplete = !reconciliation.source_complete;
                invalidated_terminal_proof_buckets = reconciliation.invalidated_bucket_start_epochs;
                unavailable_reconciliation_archive_paths =
                    reconciliation.unavailable_archive_file_paths;
            }
            Err(error) => {
                terminal_proof_reconciliation_incomplete = true;
                warn!(
                    error = %error,
                    "terminal integrity proof source reconciliation failed; keeping affected buckets untrusted"
                );
            }
        }
    }
    if terminal_proof_reconciliation_incomplete {
        // Persist the availability failure before later source reads. A subsequent error must
        // not restore ready while canonical proofs have already been revoked.
        sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, last_error = ?2, updated_at = datetime('now') WHERE id = ?3 AND NOT (status = ?4 AND datetime(updated_at) > datetime(?5))",
        )
        .bind(LONG_TERM_STATUS_ERROR)
        .bind(LONG_TERM_TERMINAL_PROOF_UNAVAILABLE_ERROR)
        .bind(LONG_TERM_STATE_ID)
        .bind(LONG_TERM_STATUS_PREPARING)
        .bind(refresh_started_at)
        .execute(pool)
        .await?;
    }
    if !unavailable_reconciliation_archive_paths.is_empty() {
        clear_long_term_invocation_replay_markers_for_unavailable_sources(
            pool,
            &unavailable_reconciliation_archive_paths,
        )
        .await?;
    }
    let mut hourly: HashMap<(i64, String, String), LongTermBucket> = HashMap::new();
    let mut daily: HashMap<(String, String, String), LongTermBucket> = HashMap::new();
    let mut statistics_start_date = previous_state
        .as_ref()
        .and_then(|state| state.statistics_start_date.clone());
    let account_identities = load_long_term_account_identities(pool).await?;
    let mut rows = Vec::new();
    let mut row_positions = HashMap::new();
    let mut processed_rows_count = 0_i64;
    if ready_state {
        let live_sql = format!(
            r#"
        SELECT
            inv.id,
            inv.invoke_id,
            inv.occurred_at,
            inv.status,
            inv.model,
            CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.requestModel') AS TEXT)), '') END AS request_model,
            CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.responseModel') AS TEXT)), '') END AS response_model,
            CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.reasoningEffort') AS TEXT)), '') END AS reasoning_effort,
            {live_upstream_account_id_sql} AS upstream_account_id,
            NULL AS upstream_account_kind,
            NULL AS upstream_account_name,
            inv.total_tokens,
            inv.output_tokens,
            inv.cost,
            inv.t_total_ms,
            inv.t_req_read_ms,
            inv.t_req_parse_ms,
            inv.t_upstream_connect_ms,
            inv.t_upstream_ttfb_ms,
            inv.t_upstream_stream_ms,
            inv.error_message
        FROM codex_invocations inv
        WHERE LOWER(TRIM(COALESCE(inv.status, ''))) NOT IN ('running', 'pending')
          AND (
              datetime(inv.occurred_at) >= datetime(?1)
              OR (
                  inv.t_total_ms IS NOT NULL
                  AND inv.t_total_ms > 0
                  AND julianday(inv.occurred_at) + inv.t_total_ms / 86400000.0 >= julianday(?1)
              )
          )
        ORDER BY inv.occurred_at ASC, inv.id ASC
            "#,
            live_upstream_account_id_sql = live_upstream_account_id_sql,
        );
        let mut live_rows = sqlx::query_as::<_, LongTermInvocationRow>(&live_sql)
            .bind(live_tail_start.as_deref())
            .fetch(pool);
        while let Some(row) = live_rows.try_next().await? {
            if row_positions.insert(row.id, rows.len()).is_none() {
                rows.push(row);
            }
        }
    } else {
        let live_sql = format!(
            r#"
        SELECT
            inv.id,
            inv.invoke_id,
            inv.occurred_at,
            inv.status,
            inv.model,
            CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.requestModel') AS TEXT)), '') END AS request_model,
            CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.responseModel') AS TEXT)), '') END AS response_model,
            CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.reasoningEffort') AS TEXT)), '') END AS reasoning_effort,
            {live_upstream_account_id_sql} AS upstream_account_id,
            NULL AS upstream_account_kind,
            NULL AS upstream_account_name,
            inv.total_tokens,
            inv.output_tokens,
            inv.cost,
            inv.t_total_ms,
            inv.t_req_read_ms,
            inv.t_req_parse_ms,
            inv.t_upstream_connect_ms,
            inv.t_upstream_ttfb_ms,
            inv.t_upstream_stream_ms,
            inv.error_message
        FROM codex_invocations inv
        WHERE LOWER(TRIM(COALESCE(inv.status, ''))) NOT IN ('running', 'pending')
        ORDER BY inv.occurred_at ASC, inv.id ASC
            "#,
            live_upstream_account_id_sql = live_upstream_account_id_sql,
        );
        let mut live_rows = sqlx::query_as::<_, LongTermInvocationRow>(&live_sql).fetch(pool);
        while let Some(row) = live_rows.try_next().await? {
            if row_positions.insert(row.id, rows.len()).is_none() {
                rows.push(row);
                processed_rows_count += 1;
            }
            if processed_rows_count % 256 == 0 {
                sqlx::query(
                    "UPDATE long_term_stats_state SET processed_rows = ?1, total_rows = ?2, updated_at = datetime('now') WHERE id = ?3",
                )
                .bind(processed_rows_count)
                // The archive workload has not been enumerated yet during a full rebuild, so
                // keep the total explicitly unknown instead of presenting a false completion
                // ratio to the preparation UI.
                .bind(0_i64)
                .bind(LONG_TERM_STATE_ID)
                .execute(pool)
                .await?;
            }
        }
    }

    let archive_paths = match load_completed_invocation_archive_paths(pool).await {
        Ok(paths) => paths,
        Err(error) if error.to_string().contains("no such table") => Vec::new(),
        Err(error) => return Err(error),
    };
    // Archive manifests update `created_at` when a legacy monthly file is rewritten. Remove
    // stale replay markers before deciding which archive files can be skipped.
    let stale_marker_cleanup = sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE target = ?1
          AND dataset = 'codex_invocations'
          AND EXISTS (
              SELECT 1
              FROM archive_batches batches
              WHERE batches.dataset = 'codex_invocations'
                AND batches.file_path = hourly_rollup_archive_replay.file_path
                AND datetime(batches.created_at) > datetime(hourly_rollup_archive_replay.replayed_at)
          )
        "#,
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .execute(pool)
    .await;
    if let Err(error) = stale_marker_cleanup
        && !error.to_string().contains("no such table")
    {
        return Err(error.into());
    }
    let replayed_archive_files = if !ready_state {
        HashSet::new()
    } else {
        match sqlx::query_scalar::<_, String>(
            r#"
        SELECT replay.file_path
        FROM hourly_rollup_archive_replay replay
        INNER JOIN archive_batches batches
          ON batches.dataset = 'codex_invocations'
         AND batches.file_path = replay.file_path
         AND batches.sha256 = replay.archive_sha256
        WHERE replay.target = ?1 AND replay.dataset = 'codex_invocations'
        "#,
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .fetch_all(pool)
        .await
        {
            Ok(rows) => rows.into_iter().collect::<HashSet<_>>(),
            Err(error) if error.to_string().contains("no such table") => HashSet::new(),
            Err(error) => return Err(error.into()),
        }
    };
    // Replayed invocation archives can still be reopened during a date rebuild, so keep the
    // attempt-account fallback available even when no archive needs first-time materialization.
    let attempt_date_range = if ready_state {
        let mut dates = HashSet::new();
        for row in &rows {
            if let Some(date) =
                parse_long_term_timestamp_ms(&row.occurred_at).and_then(|timestamp| {
                    Shanghai
                        .timestamp_millis_opt(timestamp)
                        .single()
                        .map(|value| value.date_naive())
                })
            {
                dates.insert(date);
            }
        }
        let mut requires_full_attempt_scan = false;
        for path in &archive_paths {
            if replayed_archive_files.contains(path.file_path()) {
                continue;
            }
            if path.coverage_start_at().is_none() || path.coverage_end_at().is_none() {
                requires_full_attempt_scan = true;
            } else {
                insert_long_term_date_range(
                    &mut dates,
                    path.coverage_start_at(),
                    path.coverage_end_at(),
                );
            }
        }
        if requires_full_attempt_scan {
            None
        } else {
            dates
                .iter()
                .min()
                .copied()
                .zip(dates.iter().max().copied())
                .map(|(start, end)| {
                    (
                        start.pred_opt().unwrap_or(start),
                        end.succ_opt().unwrap_or(end),
                    )
                })
        }
    } else {
        None
    };
    let (mut archive_attempt_accounts, mut attempt_archive_markers) =
        if !ready_state || attempt_date_range.is_some() {
            load_long_term_archive_attempt_accounts(pool, attempt_date_range).await?
        } else {
            (HashMap::new(), HashSet::new())
        };
    let mut archive_markers = Vec::new();
    let mut archive_read_failed = false;
    let mut failed_archive_paths = HashSet::new();
    let mut failed_archive_ranges = Vec::new();
    let mut clear_all_attempt_markers = false;
    let mut unreadable_source_start_date: Option<NaiveDate> = None;
    let mut affected_archive_dates = HashSet::new();
    let all_archive_paths = archive_paths.clone();
    for archive_path in archive_paths {
        if replayed_archive_files.contains(archive_path.file_path()) {
            continue;
        }
        let Some((archive_pool, cleanup)) = (match open_invocation_archive_batch_pool(
            &archive_path,
            "long-term-stats",
        )
        .await
        {
            Ok(value) => value,
            Err(error) => {
                archive_read_failed = true;
                failed_archive_paths.insert(archive_path.file_path().to_string());
                let unreadable_start =
                    long_term_unreadable_source_start(&archive_path, retention_start);
                unreadable_source_start_date = Some(
                    unreadable_source_start_date
                        .map_or(unreadable_start, |current| current.min(unreadable_start)),
                );
                match (
                    archive_path
                        .coverage_start_at()
                        .and_then(long_term_archive_end_date),
                    archive_path
                        .coverage_end_at()
                        .and_then(long_term_archive_end_date),
                ) {
                    (Some(start), Some(end)) => {
                        failed_archive_ranges.push((start.to_string(), end.to_string()));
                    }
                    _ => clear_all_attempt_markers = true,
                }
                warn!(error = %error, file_path = archive_path.file_path(), "long-term stats archive read failed");
                None
            }
        }) else {
            archive_read_failed = true;
            failed_archive_paths.insert(archive_path.file_path().to_string());
            let unreadable_start =
                long_term_unreadable_source_start(&archive_path, retention_start);
            unreadable_source_start_date = Some(
                unreadable_source_start_date
                    .map_or(unreadable_start, |current| current.min(unreadable_start)),
            );
            match (
                archive_path
                    .coverage_start_at()
                    .and_then(long_term_archive_end_date),
                archive_path
                    .coverage_end_at()
                    .and_then(long_term_archive_end_date),
            ) {
                (Some(start), Some(end)) => {
                    failed_archive_ranges.push((start.to_string(), end.to_string()));
                }
                _ => clear_all_attempt_markers = true,
            }
            continue;
        };
        let archive_rows = match long_term_archive_invocation_query(&archive_pool).await {
            Ok(query) => sqlx::query_as::<_, LongTermInvocationRow>(&query)
                .fetch_all(&archive_pool)
                .await
                .map_err(Into::into),
            Err(error) => Err(error),
        };
        archive_pool.close().await;
        drop(cleanup);
        match archive_rows {
            Ok(archive_rows) => {
                for mut row in archive_rows {
                    hydrate_long_term_archive_attempt_account(&mut row, &archive_attempt_accounts);
                    if let Some(date) =
                        parse_long_term_timestamp_ms(&row.occurred_at).and_then(|timestamp| {
                            Shanghai
                                .timestamp_millis_opt(timestamp)
                                .single()
                                .map(|value| value.date_naive())
                        })
                    {
                        affected_archive_dates.insert(date);
                    }
                    if let Some(index) = row_positions.get(&row.id).copied() {
                        merge_long_term_invocation_row(&mut row, &rows[index]);
                        rows[index] = row;
                    } else {
                        row_positions.insert(row.id, rows.len());
                        rows.push(row);
                        processed_rows_count += 1;
                    }
                }
                if !ready_state {
                    insert_long_term_date_range(
                        &mut affected_archive_dates,
                        archive_path.coverage_start_at(),
                        archive_path.coverage_end_at(),
                    );
                }
                archive_markers.push(archive_path.file_path().to_string());
            }
            Err(error) => {
                archive_read_failed = true;
                failed_archive_paths.insert(archive_path.file_path().to_string());
                let unreadable_start =
                    long_term_unreadable_source_start(&archive_path, retention_start);
                unreadable_source_start_date = Some(
                    unreadable_source_start_date
                        .map_or(unreadable_start, |current| current.min(unreadable_start)),
                );
                match (
                    archive_path
                        .coverage_start_at()
                        .and_then(long_term_archive_end_date),
                    archive_path
                        .coverage_end_at()
                        .and_then(long_term_archive_end_date),
                ) {
                    (Some(start), Some(end)) => {
                        failed_archive_ranges.push((start.to_string(), end.to_string()));
                    }
                    _ => clear_all_attempt_markers = true,
                }
                warn!(error = %error, file_path = archive_path.file_path(), "long-term stats archive query failed");
            }
        }
    }
    let total_rows = rows.len() as i64;
    if ready_state {
        sqlx::query(
            "UPDATE long_term_stats_state SET processed_rows = 0, total_rows = ?1, updated_at = datetime('now') WHERE id = ?2",
        )
        .bind(total_rows)
        .bind(LONG_TERM_STATE_ID)
        .execute(pool)
        .await?;
    }
    for (index, row) in rows.iter().enumerate() {
        let mut row = row.clone();
        hydrate_long_term_account_identity(&mut row, &account_identities);
        accumulate_long_term_invocation(&row, &mut hourly, &mut daily, &mut statistics_start_date);
        if ready_state && ((index + 1) % 256 == 0 || index + 1 == rows.len()) {
            sqlx::query(
                "UPDATE long_term_stats_state SET processed_rows = ?1, total_rows = ?2, updated_at = datetime('now') WHERE id = ?3",
            )
            .bind((index + 1) as i64)
            .bind(total_rows)
            .bind(LONG_TERM_STATE_ID)
            .execute(pool)
            .await?;
        }
    }

    // A completed day is audited against the canonical hourly rollup once per hour. The durable
    // source boundary only advances after a source archive is deleted with an exact interval
    // proof; a temporarily unreadable archive instead blocks candidate publication below.
    let reconstructable_start = long_term_reconstructable_start(
        retention_start,
        statistics_start_date.as_deref(),
        previous_state
            .as_ref()
            .and_then(|state| state.integrity_source_start_date.as_deref()),
    );
    for mismatch in load_long_term_reconciliation_mismatches(
        pool,
        &invalidated_terminal_proof_buckets,
        reconstructable_start,
    )
    .await?
    {
        warn!(
            stats_date = %mismatch.date,
            reason = %mismatch.reason,
            "canonical terminal proof reconciliation mismatch queued for long-term repair"
        );
        enqueue_long_term_integrity_mismatch(pool, &mismatch).await?;
    }
    if ready_state
        && integrity_audit_due
        && let Some(audit_end) = today.pred_opt()
    {
        for mismatch in audit_long_term_integrity(pool, reconstructable_start, audit_end).await? {
            warn!(
                stats_date = %mismatch.date,
                reason = %mismatch.reason,
                "long-term stats integrity mismatch detected"
            );
            enqueue_long_term_integrity_mismatch(pool, &mismatch).await?;
        }
        mark_long_term_integrity_audit(pool).await?;
    }
    let scheduled_repair_date = next_due_long_term_repair_date(pool, reconstructable_start).await?;

    // A day may be split across live rows and archive parts. Rebuild every date touched by the
    // current live tail from all overlapping source parts before replacing durable buckets.
    let mut recomputed_dates = affected_archive_dates.clone();
    for (date, _, _) in daily.keys() {
        if let Ok(date) = NaiveDate::parse_from_str(date, "%Y-%m-%d") {
            recomputed_dates.insert(date);
        }
    }
    if let Some(date) = scheduled_repair_date {
        recomputed_dates.insert(date);
    }
    let mut integrity_repair_failures = Vec::new();
    let mut completed_integrity_repairs = HashSet::new();
    if let Some(unreadable_source_start) = unreadable_source_start_date {
        if let Some(date) = scheduled_repair_date.filter(|date| *date >= unreadable_source_start) {
            let (candidate_daily, _) = long_term_candidate_integrity(date, &hourly, &daily);
            if let Some(mismatch) = queued_long_term_repair_mismatch(
                pool,
                date,
                candidate_daily,
                "one or more invocation archives are unreadable, so this repair cannot prove complete source coverage",
            )
            .await?
            {
                integrity_repair_failures.push(mismatch);
            }
        }

        // An unreadable archive can contain a request whose wall-time interval reaches any
        // later date. Do not infer a bounded continuation from manifest coverage: preserve
        // existing rows and wait for the source file to become readable again.
        let mut blocked_candidate_dates = recomputed_dates
            .iter()
            .copied()
            .filter(|date| *date >= unreadable_source_start)
            .collect::<HashSet<_>>();
        for (date, _, _) in daily.keys() {
            if let Ok(date) = NaiveDate::parse_from_str(date, "%Y-%m-%d")
                && date >= unreadable_source_start
            {
                blocked_candidate_dates.insert(date);
            }
        }
        for (bucket_start_epoch, _, _) in hourly.keys() {
            if let Some(date) = long_term_bucket_date(*bucket_start_epoch)
                && date >= unreadable_source_start
            {
                blocked_candidate_dates.insert(date);
            }
        }
        remove_long_term_candidate_dates(&mut hourly, &mut daily, &blocked_candidate_dates);
        recomputed_dates.retain(|date| !blocked_candidate_dates.contains(date));
    }
    if !ready_state && !recomputed_dates.is_empty() {
        // Initial/full rebuilds have no retained row lower bound, but completed dates still need
        // the same canonical proof before their candidate can be materialized. This also lets a
        // queued repair recover after an earlier failure left the long-term tables empty.
        let mut blocked_recomputed_dates = HashSet::new();
        for date in &recomputed_dates {
            if *date >= today {
                continue;
            }
            let (candidate_daily, candidate_hourly) =
                long_term_candidate_integrity(*date, &hourly, &daily);
            match load_long_term_integrity_oracle(pool, *date).await? {
                Some(oracle) => {
                    if let Some(mismatch) = long_term_integrity_mismatch(
                        oracle.date,
                        oracle.daily,
                        &oracle.hourly,
                        candidate_daily,
                        &candidate_hourly,
                    ) {
                        warn!(
                            stats_date = %mismatch.date,
                            reason = %mismatch.reason,
                            "long-term stats full rebuild cannot prove complete"
                        );
                        blocked_recomputed_dates.insert(*date);
                        integrity_repair_failures.push(mismatch);
                    } else if scheduled_repair_date == Some(*date) {
                        completed_integrity_repairs.insert(*date);
                    }
                }
                None if scheduled_repair_date == Some(*date) => {
                    blocked_recomputed_dates.insert(*date);
                    if let Some(mismatch) = queued_long_term_repair_mismatch(
                        pool,
                        *date,
                        candidate_daily,
                        "canonical hourly integrity evidence is unavailable for the queued repair",
                    )
                    .await?
                    {
                        blocked_recomputed_dates.insert(*date);
                        integrity_repair_failures.push(mismatch);
                    }
                }
                None => {
                    // Historical source rows are not sufficient proof by themselves. This is
                    // especially important after archive cleanup, where an incomplete source
                    // prefix can otherwise look like an empty or smaller completed day.
                    blocked_recomputed_dates.insert(*date);
                    integrity_repair_failures.push(LongTermIntegrityMismatch {
                        date: *date,
                        // There is no canonical expectation to persist yet. Keep the observed
                        // candidate for retry bookkeeping; a future trusted proof replaces it.
                        expected: candidate_daily,
                        observed: candidate_daily,
                        reason: "canonical hourly integrity evidence is unavailable for the full rebuild".to_string(),
                    });
                }
            }
        }
        remove_long_term_candidate_dates(&mut hourly, &mut daily, &blocked_recomputed_dates);
        recomputed_dates.retain(|date| !blocked_recomputed_dates.contains(date));
    }
    if ready_state && !recomputed_dates.is_empty() {
        if let Some(previous_date) = recomputed_dates
            .iter()
            .min()
            .copied()
            .and_then(|date| date.pred_opt())
        {
            recomputed_dates.insert(previous_date);
        }
        // Replacing a date can alter the preceding date when an invocation crosses midnight.
        // Load every overlapping attempt archive only after that final rebuild range is known;
        // a missing archive is a source-integrity failure, not an empty account mapping.
        if let Some((start, end)) = recomputed_dates
            .iter()
            .min()
            .copied()
            .zip(recomputed_dates.iter().max().copied())
        {
            let (rebuild_attempt_accounts, rebuild_attempt_markers) =
                load_long_term_archive_attempt_accounts(pool, Some((start, end))).await?;
            archive_attempt_accounts.extend(rebuild_attempt_accounts);
            attempt_archive_markers.extend(rebuild_attempt_markers);
        }
        let mut rebuild_rows = rows
            .iter()
            .filter(|row| {
                parse_long_term_timestamp_ms(&row.occurred_at)
                    .and_then(|timestamp| {
                        Shanghai
                            .timestamp_millis_opt(timestamp)
                            .single()
                            .map(|value| recomputed_dates.contains(&value.date_naive()))
                    })
                    .unwrap_or(false)
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut rebuild_seen_ids = rebuild_rows
            .iter()
            .map(|row| row.id)
            .collect::<HashSet<_>>();
        let min_date = recomputed_dates.iter().min().copied();
        let max_date = recomputed_dates.iter().max().copied();
        if let (Some(min_date), Some(max_date)) = (min_date, max_date) {
            let live_start = min_date.and_hms_opt(0, 0, 0).map(format_naive);
            let live_end = max_date
                .succ_opt()
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .map(format_naive);
            let live_rebuild_sql = format!(
                r#"
                SELECT
                    inv.id, inv.invoke_id, inv.occurred_at, inv.status, inv.model,
                    CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.requestModel') AS TEXT)), '') END AS request_model,
                    CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.responseModel') AS TEXT)), '') END AS response_model,
                    CASE WHEN json_valid(inv.payload) THEN NULLIF(TRIM(CAST(json_extract(inv.payload, '$.reasoningEffort') AS TEXT)), '') END AS reasoning_effort,
                    {live_upstream_account_id_sql} AS upstream_account_id,
                    NULL AS upstream_account_kind, NULL AS upstream_account_name,
                    inv.total_tokens, inv.output_tokens, inv.cost, inv.t_total_ms,
                    inv.t_req_read_ms, inv.t_req_parse_ms, inv.t_upstream_connect_ms,
                    inv.t_upstream_ttfb_ms, inv.t_upstream_stream_ms, inv.error_message
                FROM codex_invocations inv
                WHERE LOWER(TRIM(COALESCE(inv.status, ''))) NOT IN ('running', 'pending')
                  AND inv.occurred_at < ?2
                  AND (
                      inv.occurred_at >= ?1
                      OR (
                          inv.t_total_ms IS NOT NULL
                          AND inv.t_total_ms > 0
                          AND julianday(inv.occurred_at) + inv.t_total_ms / 86400000.0 >= julianday(?1)
                      )
                  )
                ORDER BY inv.occurred_at ASC, inv.id ASC
                "#,
                live_upstream_account_id_sql = live_upstream_account_id_sql,
            );
            if let (Some(live_start), Some(live_end)) = (live_start, live_end) {
                let live_rebuild_rows =
                    sqlx::query_as::<_, LongTermInvocationRow>(&live_rebuild_sql)
                        .bind(live_start)
                        .bind(live_end)
                        .fetch_all(pool)
                        .await?;
                for row in live_rebuild_rows {
                    if rebuild_seen_ids.insert(row.id) {
                        rebuild_rows.push(row);
                    }
                }
            }
        }
        for archive_path in all_archive_paths {
            let overlaps = match (
                archive_path
                    .coverage_start_at()
                    .and_then(long_term_archive_end_date),
                archive_path
                    .coverage_end_at()
                    .and_then(long_term_archive_end_date),
            ) {
                (Some(start), Some(end)) => recomputed_dates
                    .iter()
                    .any(|date| *date >= start && *date <= end),
                _ => true,
            };
            if !overlaps {
                continue;
            }
            let Some((archive_pool, cleanup)) =
                open_invocation_archive_batch_pool(&archive_path, "long-term-stats-rebuild")
                    .await?
            else {
                continue;
            };
            let archive_query = long_term_archive_invocation_query(&archive_pool).await?;
            let archive_rows = sqlx::query_as::<_, LongTermInvocationRow>(&archive_query)
                .fetch_all(&archive_pool)
                .await?;
            archive_pool.close().await;
            drop(cleanup);
            for mut row in archive_rows {
                hydrate_long_term_archive_attempt_account(&mut row, &archive_attempt_accounts);
                if rebuild_seen_ids.insert(row.id) {
                    rebuild_rows.push(row);
                }
            }
        }
        let mut rebuilt_hourly = HashMap::new();
        let mut rebuilt_daily = HashMap::new();
        let mut rebuilt_start = None;
        for mut row in rebuild_rows {
            hydrate_long_term_account_identity(&mut row, &account_identities);
            accumulate_long_term_invocation(
                &row,
                &mut rebuilt_hourly,
                &mut rebuilt_daily,
                &mut rebuilt_start,
            );
        }
        // A retained daily row is durable evidence that older source parts contributed to this
        // date. If the current archive inventory can only reproduce fewer calls, keep the
        // existing date intact instead of replacing it with a partial reconstruction. For
        // completed days, the canonical hourly rollup is the stricter completeness witness.
        let mut blocked_recomputed_dates = HashSet::new();
        for date in &recomputed_dates {
            let date_string = date.to_string();
            let (candidate_daily, candidate_hourly) =
                long_term_candidate_integrity(*date, &rebuilt_hourly, &rebuilt_daily);
            let persisted_calls = sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE(SUM(calls), 0) FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
            )
            .bind(&date_string)
            .fetch_one(pool)
            .await?;
            if *date < today {
                match load_long_term_integrity_oracle(pool, *date).await? {
                    Some(oracle) => {
                        if let Some(mismatch) = long_term_integrity_mismatch(
                            oracle.date,
                            oracle.daily,
                            &oracle.hourly,
                            candidate_daily,
                            &candidate_hourly,
                        ) {
                            warn!(
                                stats_date = %mismatch.date,
                                reason = %mismatch.reason,
                                "long-term stats rebuild cannot prove complete"
                            );
                            blocked_recomputed_dates.insert(*date);
                            integrity_repair_failures.push(mismatch);
                        } else if scheduled_repair_date == Some(*date) {
                            // Canonical zero totals are valid proof too: replace all dimensions
                            // with an empty candidate rather than leaving a stale nonzero date.
                            completed_integrity_repairs.insert(*date);
                        }
                    }
                    None => {
                        blocked_recomputed_dates.insert(*date);
                        if scheduled_repair_date == Some(*date)
                            && let Some(mismatch) = queued_long_term_repair_mismatch(
                                pool,
                                *date,
                                candidate_daily,
                                "canonical hourly integrity evidence is unavailable for the queued repair",
                            )
                            .await?
                        {
                            integrity_repair_failures.push(mismatch);
                        }
                    }
                }
                continue;
            }
            if persisted_calls > candidate_daily.calls {
                blocked_recomputed_dates.insert(*date);
                if scheduled_repair_date == Some(*date)
                    && let Some(mismatch) = queued_long_term_repair_mismatch(
                        pool,
                        *date,
                        candidate_daily,
                        format!(
                            "candidate calls={} is below retained calls={persisted_calls}; source reconstruction is incomplete",
                            candidate_daily.calls
                        ),
                    )
                    .await?
                {
                    integrity_repair_failures.push(mismatch);
                }
            } else if scheduled_repair_date == Some(*date)
                && let Some(mismatch) = queued_long_term_repair_mismatch(
                    pool,
                    *date,
                    candidate_daily,
                    "canonical hourly integrity evidence is unavailable for the queued repair",
                )
                .await?
            {
                blocked_recomputed_dates.insert(*date);
                integrity_repair_failures.push(mismatch);
            }
        }
        if !blocked_recomputed_dates.is_empty() {
            remove_long_term_candidate_dates(
                &mut rebuilt_hourly,
                &mut rebuilt_daily,
                &blocked_recomputed_dates,
            );
            // The partial live candidate was built before the full-date reconstruction. Drop it
            // as well, otherwise its UPSERT would still overwrite the durable rollup.
            remove_long_term_candidate_dates(&mut hourly, &mut daily, &blocked_recomputed_dates);
            recomputed_dates.retain(|date| !blocked_recomputed_dates.contains(date));
        }
        hourly.retain(|(bucket_start, _, _), _| {
            Shanghai
                .timestamp_opt(*bucket_start, 0)
                .single()
                .map(|value| !recomputed_dates.contains(&value.date_naive()))
                .unwrap_or(true)
        });
        daily.retain(|(date, _, _), _| {
            NaiveDate::parse_from_str(date, "%Y-%m-%d")
                .map(|value| !recomputed_dates.contains(&value))
                .unwrap_or(true)
        });
        hourly.extend(rebuilt_hourly);
        daily.extend(rebuilt_daily);
        if let Some(rebuilt_start) = rebuilt_start
            && statistics_start_date
                .as_deref()
                .is_none_or(|current| rebuilt_start.as_str() < current)
        {
            statistics_start_date = Some(rebuilt_start);
        }
    }

    // Daily rows are permanent. Hourly rows are incrementally refreshed so replayed archive
    // buckets remain available after their source files are cleaned up.
    let mut tx = pool.begin().await?;
    let has_persisted_daily_rows =
        sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM long_term_usage_daily LIMIT 1)")
            .fetch_one(&mut *tx)
            .await?
            != 0;
    if ready_state {
        for date in recomputed_dates {
            sqlx::query("DELETE FROM long_term_usage_daily WHERE stats_date = ?1")
                .bind(date.to_string())
                .execute(&mut *tx)
                .await?;
            let day_start_epoch = date
                .and_hms_opt(0, 0, 0)
                .and_then(|value| Shanghai.from_local_datetime(&value).single())
                .map(|value| value.timestamp());
            let day_end_epoch = date
                .succ_opt()
                .and_then(|value| value.and_hms_opt(0, 0, 0))
                .and_then(|value| Shanghai.from_local_datetime(&value).single())
                .map(|value| value.timestamp());
            if let (Some(day_start_epoch), Some(day_end_epoch)) = (day_start_epoch, day_end_epoch) {
                sqlx::query(
                    "DELETE FROM long_term_usage_hourly WHERE bucket_start_epoch >= ?1 AND bucket_start_epoch < ?2",
                )
                .bind(day_start_epoch)
                .bind(day_end_epoch)
                .execute(&mut *tx)
                .await?;
            }
        }
    } else if !recomputed_dates.is_empty() {
        // A full rebuild may change a grouping key (for example, after reasoning-effort
        // backfill). Replace every bucket covered by readable sources so superseded keys cannot
        // remain and double-count the same invocation. Dates without readable source coverage
        // are intentionally preserved for archive-retention continuity.
        for date in &recomputed_dates {
            sqlx::query("DELETE FROM long_term_usage_daily WHERE stats_date = ?1")
                .bind(date.to_string())
                .execute(&mut *tx)
                .await?;
            let day_start_epoch = date
                .and_hms_opt(0, 0, 0)
                .and_then(|value| Shanghai.from_local_datetime(&value).single())
                .map(|value| value.timestamp());
            let day_end_epoch = date
                .succ_opt()
                .and_then(|value| value.and_hms_opt(0, 0, 0))
                .and_then(|value| Shanghai.from_local_datetime(&value).single())
                .map(|value| value.timestamp());
            if let (Some(day_start_epoch), Some(day_end_epoch)) = (day_start_epoch, day_end_epoch) {
                sqlx::query(
                    "DELETE FROM long_term_usage_hourly WHERE bucket_start_epoch >= ?1 AND bucket_start_epoch < ?2",
                )
                .bind(day_start_epoch)
                .bind(day_end_epoch)
                .execute(&mut *tx)
                .await?;
            }
        }
    } else if !has_persisted_daily_rows {
        sqlx::query("DELETE FROM long_term_usage_hourly")
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM long_term_usage_daily")
            .execute(&mut *tx)
            .await?;
    }
    let retention_start_epoch = retention_start
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Shanghai.from_local_datetime(&value).single())
        .map(|value| value.timestamp())
        .unwrap_or(i64::MIN);
    sqlx::query("DELETE FROM long_term_usage_hourly WHERE bucket_start_epoch < ?1")
        .bind(retention_start_epoch)
        .execute(&mut *tx)
        .await?;

    for bucket in hourly.values().filter(|bucket| {
        Shanghai
            .timestamp_opt(bucket.bucket_start_epoch, 0)
            .single()
            .map(|value| value.date_naive() >= retention_start)
            .unwrap_or(false)
    }) {
        insert_long_term_hourly(&mut tx, bucket).await?;
    }
    for bucket in daily.values() {
        insert_long_term_daily(&mut tx, bucket).await?;
    }
    for mismatch in &integrity_repair_failures {
        schedule_long_term_repair_retry(&mut tx, mismatch).await?;
    }
    for date in &completed_integrity_repairs {
        sqlx::query("DELETE FROM long_term_stats_repair_queue WHERE stats_date = ?1")
            .bind(date.to_string())
            .execute(&mut *tx)
            .await?;
        info!(stats_date = %date, "long-term stats integrity repair completed");
    }
    let pending_integrity_repairs =
        count_reconstructable_long_term_repairs(&mut tx, reconstructable_start).await?;
    let statistics_start_date =
        statistics_start_date.and_then(|date| NaiveDate::parse_from_str(&date, "%Y-%m-%d").ok());
    // Any unreadable source prevents a complete replacement proof. Existing materialized rows
    // remain untouched, and the existing API error state keeps potentially incomplete data out
    // of view until the next source retry can reconcile it.
    let status = if pending_integrity_repairs > 0
        || archive_read_failed
        || terminal_proof_reconciliation_incomplete
    {
        LONG_TERM_STATUS_ERROR
    } else if rows.is_empty() && daily.is_empty() && !has_persisted_daily_rows {
        LONG_TERM_STATUS_EMPTY
    } else {
        LONG_TERM_STATUS_READY
    };
    let last_error = if terminal_proof_reconciliation_incomplete {
        // Preserve the retry trigger while a queued repair waits for the canonical proof to be
        // reconciled again. Without it, a backoff window could leave revoked proof buckets
        // unexamined until the next hourly audit.
        Some(LONG_TERM_TERMINAL_PROOF_UNAVAILABLE_ERROR.to_string())
    } else if pending_integrity_repairs > 0 {
        Some(format!(
            "long-term integrity repair pending for {pending_integrity_repairs} date(s)"
        ))
    } else {
        archive_read_failed
            .then_some("one or more invocation archives could not be materialized".to_string())
    };
    sqlx::query(
        "UPDATE long_term_stats_state SET status = ?1, statistics_start_date = ?2, processed_rows = ?3, total_rows = ?3, last_error = ?4, updated_at = datetime('now') WHERE id = ?5 AND NOT (status = ?6 AND datetime(updated_at) > datetime(?7))",
    )
    .bind(status)
    .bind(statistics_start_date.map(|date| date.to_string()))
    .bind(if ready_state { rows.len() as i64 } else { processed_rows_count })
    .bind(last_error)
    .bind(LONG_TERM_STATE_ID)
    .bind(LONG_TERM_STATUS_PREPARING)
    .bind(refresh_started_at)
    .execute(&mut *tx)
    .await?;
    for file_path in archive_markers {
        let archive_sha256 = sqlx::query_scalar::<_, String>(
            "SELECT sha256 FROM archive_batches WHERE dataset = 'codex_invocations' AND file_path = ?1 LIMIT 1",
        )
        .bind(&file_path)
        .fetch_optional(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT OR REPLACE INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'codex_invocations', ?2, ?3)",
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .bind(file_path)
        .bind(archive_sha256)
        .execute(&mut *tx)
        .await?;
    }
    if archive_read_failed {
        for failed_archive_path in failed_archive_paths {
            sqlx::query(
                "DELETE FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = 'codex_invocations' AND file_path = ?2",
            )
            .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
            .bind(failed_archive_path)
            .execute(&mut *tx)
            .await?;
        }
        if clear_all_attempt_markers {
            sqlx::query(
                "DELETE FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = 'pool_upstream_request_attempts'",
            )
            .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
            .execute(&mut *tx)
            .await?;
        } else {
            for (failed_start, failed_end) in failed_archive_ranges {
                sqlx::query(
                    r#"
                    DELETE FROM hourly_rollup_archive_replay
                    WHERE target = ?1
                      AND dataset = 'pool_upstream_request_attempts'
                      AND EXISTS (
                          SELECT 1
                          FROM archive_batches attempts
                          WHERE attempts.dataset = 'pool_upstream_request_attempts'
                            AND attempts.file_path = hourly_rollup_archive_replay.file_path
                            AND (attempts.coverage_end_at IS NULL OR attempts.coverage_end_at >= ?2)
                            AND (attempts.coverage_start_at IS NULL OR attempts.coverage_start_at <= ?3)
                      )
                    "#,
                )
                .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
                .bind(failed_start)
                .bind(failed_end)
                .execute(&mut *tx)
                .await?;
            }
        }
    } else {
        for (file_path, archive_sha256) in attempt_archive_markers {
            sqlx::query(
                "INSERT OR REPLACE INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'pool_upstream_request_attempts', ?2, ?3)",
            )
            .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
            .bind(file_path)
            .bind(archive_sha256)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Clone, FromRow)]
struct LongTermAccountIdentity {
    id: i64,
    kind: String,
    display_name: String,
}

async fn load_long_term_account_identities(
    pool: &Pool<Sqlite>,
) -> Result<HashMap<i64, LongTermAccountIdentity>> {
    let result = sqlx::query_as::<_, LongTermAccountIdentity>(
        "SELECT id, kind, display_name FROM pool_upstream_accounts",
    )
    .fetch_all(pool)
    .await;
    match result {
        Ok(rows) => Ok(rows
            .into_iter()
            .map(|identity| (identity.id, identity))
            .collect()),
        Err(error) if error.to_string().contains("no such table") => Ok(HashMap::new()),
        Err(error) => Err(error.into()),
    }
}

fn long_term_archive_end_date(raw: &str) -> Option<NaiveDate> {
    parse_long_term_timestamp_ms(raw).and_then(|timestamp| {
        Shanghai
            .timestamp_millis_opt(timestamp)
            .single()
            .map(|value| value.date_naive())
    })
}

fn insert_long_term_date_range(
    dates: &mut HashSet<NaiveDate>,
    start: Option<&str>,
    end: Option<&str>,
) {
    let (Some(start), Some(end)) = (
        start.and_then(long_term_archive_end_date),
        end.and_then(long_term_archive_end_date),
    ) else {
        return;
    };
    let mut date = start;
    while date <= end {
        dates.insert(date);
        let Some(next) = date.succ_opt() else {
            break;
        };
        date = next;
    }
}

fn hydrate_long_term_account_identity(
    row: &mut LongTermInvocationRow,
    account_identities: &HashMap<i64, LongTermAccountIdentity>,
) {
    if row.upstream_account_kind.is_none()
        && let Some(account_id) = row.upstream_account_id
        && let Some(identity) = account_identities.get(&account_id)
    {
        row.upstream_account_kind = Some(identity.kind.clone());
        row.upstream_account_name = Some(identity.display_name.clone());
    }
}

fn merge_long_term_invocation_row(
    preferred: &mut LongTermInvocationRow,
    fallback: &LongTermInvocationRow,
) {
    if preferred.invoke_id.is_none() {
        preferred.invoke_id = fallback.invoke_id.clone();
    }
    if preferred.occurred_at.is_empty() {
        preferred.occurred_at.clone_from(&fallback.occurred_at);
    }
    if preferred.status.is_none() {
        preferred.status = fallback.status.clone();
    }
    if preferred.model.is_none() {
        preferred.model = fallback.model.clone();
    }
    if preferred.request_model.is_none() {
        preferred.request_model = fallback.request_model.clone();
    }
    if preferred.response_model.is_none() {
        preferred.response_model = fallback.response_model.clone();
    }
    if preferred.reasoning_effort.is_none() {
        preferred.reasoning_effort = fallback.reasoning_effort.clone();
    }
    if preferred.upstream_account_id.is_none() {
        preferred.upstream_account_id = fallback.upstream_account_id;
    }
    if preferred.upstream_account_kind.is_none() {
        preferred.upstream_account_kind = fallback.upstream_account_kind.clone();
    }
    if preferred.upstream_account_name.is_none() {
        preferred.upstream_account_name = fallback.upstream_account_name.clone();
    }
    if preferred.total_tokens.is_none() {
        preferred.total_tokens = fallback.total_tokens;
    }
    if preferred.output_tokens.is_none() {
        preferred.output_tokens = fallback.output_tokens;
    }
    if preferred.cost.is_none() {
        preferred.cost = fallback.cost;
    }
    if preferred.t_total_ms.is_none() {
        preferred.t_total_ms = fallback.t_total_ms;
    }
    if preferred.t_req_read_ms.is_none() {
        preferred.t_req_read_ms = fallback.t_req_read_ms;
    }
    if preferred.t_req_parse_ms.is_none() {
        preferred.t_req_parse_ms = fallback.t_req_parse_ms;
    }
    if preferred.t_upstream_connect_ms.is_none() {
        preferred.t_upstream_connect_ms = fallback.t_upstream_connect_ms;
    }
    if preferred.t_upstream_ttfb_ms.is_none() {
        preferred.t_upstream_ttfb_ms = fallback.t_upstream_ttfb_ms;
    }
    if preferred.t_upstream_stream_ms.is_none() {
        preferred.t_upstream_stream_ms = fallback.t_upstream_stream_ms;
    }
    if preferred.error_message.is_none() {
        preferred.error_message = fallback.error_message.clone();
    }
}

fn accumulate_long_term_invocation(
    row: &LongTermInvocationRow,
    hourly: &mut HashMap<(i64, String, String), LongTermBucket>,
    daily: &mut HashMap<(String, String, String), LongTermBucket>,
    statistics_start_date: &mut Option<String>,
) {
    let Some(start_ms) = parse_long_term_timestamp_ms(&row.occurred_at) else {
        return;
    };
    let Some(local_date) = Shanghai
        .timestamp_millis_opt(start_ms)
        .single()
        .map(|value| value.date_naive())
    else {
        return;
    };
    let date_string = local_date.to_string();
    if statistics_start_date
        .as_deref()
        .is_none_or(|current| date_string.as_str() < current)
    {
        *statistics_start_date = Some(date_string.clone());
    }
    let model = normalize_long_term_model(row);
    let reasoning = normalize_long_term_reasoning(row.reasoning_effort.as_deref());
    let upstream = normalize_long_term_upstream(row);
    let dimensions = [
        (
            "overall",
            "overall".to_string(),
            "全部".to_string(),
            String::new(),
        ),
        (
            "model",
            long_term_model_series_key(&model, &reasoning),
            model,
            reasoning,
        ),
        ("upstream", upstream.0, upstream.1, String::new()),
    ];
    let interval_end_ms = row
        .t_total_ms
        .filter(|value| value.is_finite() && *value > 0.0)
        .map(|value| start_ms + value.round() as i64);
    for (dimension, series_key, display_name, reasoning_effort) in dimensions {
        let interval = if is_success_status(row.status.as_deref(), row.error_message.as_deref()) {
            interval_end_ms.map(|end_ms| (start_ms, end_ms))
        } else {
            None
        };
        add_long_term_row(
            hourly,
            dimension,
            &series_key,
            &display_name,
            &reasoning_effort,
            start_ms,
            row,
            interval,
        );
        add_long_term_daily_row(
            daily,
            dimension,
            &series_key,
            &display_name,
            &reasoning_effort,
            &date_string,
            row,
            interval,
        );
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "The bucket key and invocation timing are kept explicit at this aggregation boundary."
)]
fn add_long_term_row(
    buckets: &mut HashMap<(i64, String, String), LongTermBucket>,
    dimension: &str,
    series_key: &str,
    display_name: &str,
    reasoning_effort: &str,
    start_ms: i64,
    row: &LongTermInvocationRow,
    interval: Option<(i64, i64)>,
) {
    let hour_start_ms = start_ms.div_euclid(LONG_TERM_HOUR_MS) * LONG_TERM_HOUR_MS;
    let original_interval = interval;
    let interval = interval.and_then(|(start, end)| {
        let hour_end = hour_start_ms + LONG_TERM_HOUR_MS;
        let clipped_start = start.max(hour_start_ms);
        let clipped_end = end.min(hour_end);
        (clipped_end > clipped_start).then_some((clipped_start, clipped_end))
    });
    let key = (
        hour_start_ms / 1000,
        dimension.to_string(),
        series_key.to_string(),
    );
    let bucket = buckets.entry(key).or_insert_with(|| LongTermBucket {
        bucket_start_epoch: hour_start_ms / 1000,
        dimension: dimension.to_string(),
        series_key: series_key.to_string(),
        display_name: display_name.to_string(),
        reasoning_effort: reasoning_effort.to_string(),
        stats_date: None,
        accumulator: LongTermAccumulator::default(),
    });
    bucket.accumulator.add_call(row, interval);
    let Some((interval_start, interval_end)) = original_interval else {
        return;
    };
    let mut next_hour_start = hour_start_ms + LONG_TERM_HOUR_MS;
    while next_hour_start < interval_end {
        let segment_start = next_hour_start.max(interval_start);
        let segment_end = (next_hour_start + LONG_TERM_HOUR_MS).min(interval_end);
        if segment_end > segment_start {
            let key = (
                next_hour_start / 1000,
                dimension.to_string(),
                series_key.to_string(),
            );
            let bucket = buckets.entry(key).or_insert_with(|| LongTermBucket {
                bucket_start_epoch: next_hour_start / 1000,
                dimension: dimension.to_string(),
                series_key: series_key.to_string(),
                display_name: display_name.to_string(),
                reasoning_effort: reasoning_effort.to_string(),
                stats_date: None,
                accumulator: LongTermAccumulator::default(),
            });
            bucket
                .accumulator
                .add_interval(Some((segment_start, segment_end)));
        }
        next_hour_start += LONG_TERM_HOUR_MS;
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "The bucket key and invocation timing are kept explicit at this aggregation boundary."
)]
fn add_long_term_daily_row(
    buckets: &mut HashMap<(String, String, String), LongTermBucket>,
    dimension: &str,
    series_key: &str,
    display_name: &str,
    reasoning_effort: &str,
    date: &str,
    row: &LongTermInvocationRow,
    interval: Option<(i64, i64)>,
) {
    let original_interval = interval;
    let key = (
        date.to_string(),
        dimension.to_string(),
        series_key.to_string(),
    );
    let bucket = buckets.entry(key).or_insert_with(|| LongTermBucket {
        bucket_start_epoch: 0,
        dimension: dimension.to_string(),
        series_key: series_key.to_string(),
        display_name: display_name.to_string(),
        reasoning_effort: reasoning_effort.to_string(),
        stats_date: Some(date.to_string()),
        accumulator: LongTermAccumulator::default(),
    });
    let interval = interval.and_then(|(start, end)| {
        let day_start = NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .ok()?
            .and_hms_opt(0, 0, 0)
            .and_then(|value| Shanghai.from_local_datetime(&value).single())?;
        let day_end = day_start.checked_add_signed(ChronoDuration::days(1))?;
        let clipped_start = start.max(day_start.timestamp_millis());
        let clipped_end = end.min(day_end.timestamp_millis());
        (clipped_end > clipped_start).then_some((clipped_start, clipped_end))
    });
    bucket.accumulator.add_call(row, interval);
    let Some((interval_start, interval_end)) = original_interval else {
        return;
    };
    let Some(mut next_date) = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .ok()
        .and_then(|value| value.succ_opt())
    else {
        return;
    };
    loop {
        let Some(day_start) = next_date
            .and_hms_opt(0, 0, 0)
            .and_then(|value| Shanghai.from_local_datetime(&value).single())
        else {
            return;
        };
        if day_start.timestamp_millis() >= interval_end {
            break;
        }
        let day_end = day_start
            .checked_add_signed(ChronoDuration::days(1))
            .map(|value| value.timestamp_millis())
            .unwrap_or(interval_end);
        let segment_start = interval_start.max(day_start.timestamp_millis());
        let segment_end = interval_end.min(day_end);
        if segment_end > segment_start {
            let date_string = next_date.to_string();
            let key = (
                date_string.clone(),
                dimension.to_string(),
                series_key.to_string(),
            );
            let bucket = buckets.entry(key).or_insert_with(|| LongTermBucket {
                bucket_start_epoch: 0,
                dimension: dimension.to_string(),
                series_key: series_key.to_string(),
                display_name: display_name.to_string(),
                reasoning_effort: reasoning_effort.to_string(),
                stats_date: Some(date_string),
                accumulator: LongTermAccumulator::default(),
            });
            bucket
                .accumulator
                .add_interval(Some((segment_start, segment_end)));
        }
        if segment_end >= interval_end {
            break;
        }
        let Some(next) = next_date.succ_opt() else {
            break;
        };
        next_date = next;
    }
}

fn projection_interval_key(
    bucket_kind: &'static str,
    bucket_key: String,
    dimension: String,
    series_key: String,
) -> LongTermProjectionIntervalKey {
    LongTermProjectionIntervalKey {
        bucket_kind,
        bucket_key,
        dimension,
        series_key,
    }
}

fn projection_bucket_date_for_hour(bucket_start_epoch: i64) -> Option<String> {
    Shanghai
        .timestamp_opt(bucket_start_epoch, 0)
        .single()
        .map(|timestamp| timestamp.date_naive().to_string())
}

fn collect_long_term_projection_interval_segments(
    hourly: &HashMap<(i64, String, String), LongTermBucket>,
    daily: &HashMap<(String, String, String), LongTermBucket>,
    invocation_row_id: i64,
) -> Vec<LongTermProjectionIntervalSegment> {
    let mut segments = Vec::new();
    for bucket in hourly.values() {
        let Some(bucket_date) = projection_bucket_date_for_hour(bucket.bucket_start_epoch) else {
            continue;
        };
        let key = projection_interval_key(
            "hourly",
            bucket.bucket_start_epoch.to_string(),
            bucket.dimension.clone(),
            bucket.series_key.clone(),
        );
        for &(interval_start_ms, interval_end_ms) in &bucket.accumulator.intervals {
            segments.push(LongTermProjectionIntervalSegment {
                key: key.clone(),
                bucket_date: bucket_date.clone(),
                invocation_row_id,
                interval_start_ms,
                interval_end_ms,
            });
        }
    }
    for bucket in daily.values() {
        let Some(bucket_date) = bucket.stats_date.clone() else {
            continue;
        };
        let key = projection_interval_key(
            "daily",
            bucket_date.clone(),
            bucket.dimension.clone(),
            bucket.series_key.clone(),
        );
        for &(interval_start_ms, interval_end_ms) in &bucket.accumulator.intervals {
            segments.push(LongTermProjectionIntervalSegment {
                key: key.clone(),
                bucket_date: bucket_date.clone(),
                invocation_row_id,
                interval_start_ms,
                interval_end_ms,
            });
        }
    }
    segments
}

fn merge_long_term_projection_bucket(target: &mut LongTermBucket, source: &LongTermBucket) {
    target.accumulator.merge(&source.accumulator);
}

fn merge_long_term_projection_buckets<K>(
    target: &mut HashMap<K, LongTermBucket>,
    source: HashMap<K, LongTermBucket>,
) where
    K: Eq + Hash,
{
    for (key, bucket) in source {
        match target.entry(key) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                merge_long_term_projection_bucket(entry.get_mut(), &bucket);
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(bucket);
            }
        }
    }
}

async fn insert_long_term_hourly(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    bucket: &LongTermBucket,
) -> Result<()> {
    insert_long_term_rollup(
        tx,
        "long_term_usage_hourly",
        "bucket_start_epoch",
        bucket.bucket_start_epoch.to_string(),
        bucket,
    )
    .await
}

async fn insert_long_term_daily(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    bucket: &LongTermBucket,
) -> Result<()> {
    let Some(date) = bucket.stats_date.as_deref() else {
        return Ok(());
    };
    insert_long_term_rollup(
        tx,
        "long_term_usage_daily",
        "stats_date",
        date.to_string(),
        bucket,
    )
    .await
}

async fn insert_long_term_rollup(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    table: &str,
    bucket_column: &str,
    bucket_value: String,
    bucket: &LongTermBucket,
) -> Result<()> {
    let conflict_target = if table == "long_term_usage_daily" {
        "stats_date, dimension, series_key"
    } else {
        "bucket_start_epoch, dimension, series_key"
    };
    let sql = format!(
        "INSERT INTO {table} ({bucket_column}, dimension, series_key, display_name, reasoning_effort, calls, token_total, token_samples, cost_total, cost_samples, usage_time_ms, usage_time_samples, wall_time_ms, wall_time_samples, output_tokens_total, stream_duration_ms, output_speed_samples, first_byte_sum_ms, first_byte_samples, response_sum_ms, response_samples) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21) ON CONFLICT ({conflict_target}) DO UPDATE SET display_name = excluded.display_name, reasoning_effort = excluded.reasoning_effort, calls = excluded.calls, token_total = excluded.token_total, token_samples = excluded.token_samples, cost_total = excluded.cost_total, cost_samples = excluded.cost_samples, usage_time_ms = excluded.usage_time_ms, usage_time_samples = excluded.usage_time_samples, wall_time_ms = excluded.wall_time_ms, wall_time_samples = excluded.wall_time_samples, output_tokens_total = excluded.output_tokens_total, stream_duration_ms = excluded.stream_duration_ms, output_speed_samples = excluded.output_speed_samples, first_byte_sum_ms = excluded.first_byte_sum_ms, first_byte_samples = excluded.first_byte_samples, response_sum_ms = excluded.response_sum_ms, response_samples = excluded.response_samples, updated_at = datetime('now')"
    );
    let acc = &bucket.accumulator;
    sqlx::query(&sql)
        .bind(bucket_value)
        .bind(&bucket.dimension)
        .bind(&bucket.series_key)
        .bind(&bucket.display_name)
        .bind(&bucket.reasoning_effort)
        .bind(acc.calls)
        .bind(acc.token_total)
        .bind(acc.token_samples)
        .bind(acc.cost_total)
        .bind(acc.cost_samples)
        .bind(acc.usage_time_ms)
        .bind(acc.usage_time_samples)
        .bind(acc.wall_time_ms())
        .bind(acc.wall_sample_count())
        .bind(acc.output_tokens_total)
        .bind(acc.stream_duration_ms)
        .bind(acc.output_speed_samples)
        .bind(acc.first_byte_sum_ms)
        .bind(acc.first_byte_samples)
        .bind(acc.response_sum_ms)
        .bind(acc.response_samples)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn load_long_term_projection_interval_index(
    pool: &Pool<Sqlite>,
    dates: &HashSet<String>,
) -> Result<HashMap<LongTermProjectionIntervalKey, LongTermProjectionIntervalUnion>> {
    if dates.is_empty() {
        return Ok(HashMap::new());
    }
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT bucket_kind, bucket_key, dimension, series_key, interval_start_ms, interval_end_ms FROM long_term_projection_intervals WHERE bucket_date IN (",
    );
    let mut separated = builder.separated(", ");
    for date in dates {
        separated.push_bind(date);
    }
    separated.push_unseparated(")");
    let rows = builder
        .build_query_as::<LongTermProjectionIntervalRow>()
        .fetch_all(pool)
        .await?;
    let mut index = HashMap::new();
    for row in rows {
        let bucket_kind = match row.bucket_kind.as_str() {
            "hourly" => "hourly",
            "daily" => "daily",
            _ => continue,
        };
        index
            .entry(projection_interval_key(
                bucket_kind,
                row.bucket_key,
                row.dimension,
                row.series_key,
            ))
            .or_insert_with(LongTermProjectionIntervalUnion::default)
            .add(row.interval_start_ms, row.interval_end_ms);
    }
    Ok(index)
}

async fn insert_long_term_projection_interval_segments(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    segments: &[LongTermProjectionIntervalSegment],
    interval_index: &mut HashMap<LongTermProjectionIntervalKey, LongTermProjectionIntervalUnion>,
) -> Result<()> {
    for segment in segments {
        let result = sqlx::query(
            "INSERT OR IGNORE INTO long_term_projection_intervals (bucket_kind, bucket_date, bucket_key, dimension, series_key, invocation_row_id, interval_start_ms, interval_end_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(segment.key.bucket_kind)
        .bind(&segment.bucket_date)
        .bind(&segment.key.bucket_key)
        .bind(&segment.key.dimension)
        .bind(&segment.key.series_key)
        .bind(segment.invocation_row_id)
        .bind(segment.interval_start_ms)
        .bind(segment.interval_end_ms)
        .execute(&mut **tx)
        .await?;
        if result.rows_affected() > 0 {
            interval_index
                .entry(segment.key.clone())
                .or_default()
                .add(segment.interval_start_ms, segment.interval_end_ms);
        }
    }
    Ok(())
}

async fn merge_long_term_projection_rollup(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    table: &str,
    bucket_column: &str,
    bucket_value: String,
    bucket: &LongTermBucket,
    interval_union: Option<&LongTermProjectionIntervalUnion>,
) -> Result<()> {
    let conflict_target = if table == "long_term_usage_daily" {
        "stats_date, dimension, series_key"
    } else {
        "bucket_start_epoch, dimension, series_key"
    };
    let sql = format!(
        "INSERT INTO {table} ({bucket_column}, dimension, series_key, display_name, reasoning_effort, calls, token_total, token_samples, cost_total, cost_samples, usage_time_ms, usage_time_samples, wall_time_ms, wall_time_samples, output_tokens_total, stream_duration_ms, output_speed_samples, first_byte_sum_ms, first_byte_samples, response_sum_ms, response_samples) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21) ON CONFLICT ({conflict_target}) DO UPDATE SET display_name = excluded.display_name, reasoning_effort = excluded.reasoning_effort, calls = calls + excluded.calls, token_total = token_total + excluded.token_total, token_samples = token_samples + excluded.token_samples, cost_total = cost_total + excluded.cost_total, cost_samples = cost_samples + excluded.cost_samples, usage_time_ms = usage_time_ms + excluded.usage_time_ms, usage_time_samples = usage_time_samples + excluded.usage_time_samples, wall_time_ms = excluded.wall_time_ms, wall_time_samples = excluded.wall_time_samples, output_tokens_total = output_tokens_total + excluded.output_tokens_total, stream_duration_ms = stream_duration_ms + excluded.stream_duration_ms, output_speed_samples = output_speed_samples + excluded.output_speed_samples, first_byte_sum_ms = first_byte_sum_ms + excluded.first_byte_sum_ms, first_byte_samples = first_byte_samples + excluded.first_byte_samples, response_sum_ms = response_sum_ms + excluded.response_sum_ms, response_samples = response_samples + excluded.response_samples, updated_at = datetime('now')"
    );
    let acc = &bucket.accumulator;
    sqlx::query(&sql)
        .bind(bucket_value)
        .bind(&bucket.dimension)
        .bind(&bucket.series_key)
        .bind(&bucket.display_name)
        .bind(&bucket.reasoning_effort)
        .bind(acc.calls)
        .bind(acc.token_total)
        .bind(acc.token_samples)
        .bind(acc.cost_total)
        .bind(acc.cost_samples)
        .bind(acc.usage_time_ms)
        .bind(acc.usage_time_samples)
        .bind(interval_union.map_or(0, |value| value.duration_ms) as f64)
        .bind(interval_union.map_or(0, |value| value.sample_count))
        .bind(acc.output_tokens_total)
        .bind(acc.stream_duration_ms)
        .bind(acc.output_speed_samples)
        .bind(acc.first_byte_sum_ms)
        .bind(acc.first_byte_samples)
        .bind(acc.response_sum_ms)
        .bind(acc.response_samples)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn apply_long_term_projection_incremental(
    state: &AppState,
    hourly: &HashMap<(i64, String, String), LongTermBucket>,
    daily: &HashMap<(String, String, String), LongTermBucket>,
    segments: &[LongTermProjectionIntervalSegment],
    next_cursor: i64,
    event_count: usize,
) -> Result<()> {
    apply_long_term_projection_incremental_with_runtime(
        &state.pool,
        &state.long_term_projection_runtime,
        hourly,
        daily,
        segments,
        next_cursor,
        event_count,
    )
    .await
}

async fn apply_long_term_projection_incremental_with_runtime(
    pool: &Pool<Sqlite>,
    runtime: &Arc<Mutex<LongTermProjectionRuntime>>,
    hourly: &HashMap<(i64, String, String), LongTermBucket>,
    daily: &HashMap<(String, String, String), LongTermBucket>,
    segments: &[LongTermProjectionIntervalSegment],
    next_cursor: i64,
    event_count: usize,
) -> Result<()> {
    let mut dates = HashSet::new();
    for segment in segments {
        dates.insert(segment.bucket_date.clone());
    }
    for bucket in daily.values() {
        if let Some(date) = bucket.stats_date.as_ref() {
            dates.insert(date.clone());
        }
    }
    let (mut interval_index, mut loaded_dates) = {
        let runtime = runtime.lock().await;
        (
            runtime.interval_index.clone(),
            runtime.loaded_interval_dates.clone(),
        )
    };
    let missing_dates = dates
        .difference(&loaded_dates)
        .cloned()
        .collect::<HashSet<_>>();
    if !missing_dates.is_empty() {
        interval_index
            .extend(load_long_term_projection_interval_index(pool, &missing_dates).await?);
        loaded_dates.extend(missing_dates);
    }

    let mut tx = pool.begin().await?;
    insert_long_term_projection_interval_segments(&mut tx, segments, &mut interval_index).await?;
    for bucket in hourly.values() {
        let key = projection_interval_key(
            "hourly",
            bucket.bucket_start_epoch.to_string(),
            bucket.dimension.clone(),
            bucket.series_key.clone(),
        );
        merge_long_term_projection_rollup(
            &mut tx,
            "long_term_usage_hourly",
            "bucket_start_epoch",
            bucket.bucket_start_epoch.to_string(),
            bucket,
            interval_index.get(&key),
        )
        .await?;
    }
    for bucket in daily.values() {
        let Some(date) = bucket.stats_date.clone() else {
            continue;
        };
        let key = projection_interval_key(
            "daily",
            date.clone(),
            bucket.dimension.clone(),
            bucket.series_key.clone(),
        );
        merge_long_term_projection_rollup(
            &mut tx,
            "long_term_usage_daily",
            "stats_date",
            date,
            bucket,
            interval_index.get(&key),
        )
        .await?;
    }
    sqlx::query(
        "INSERT INTO long_term_projection_state (consumer, cursor_row_id, last_flush_at, last_error) VALUES (?1, ?2, datetime('now'), NULL) ON CONFLICT(consumer) DO UPDATE SET cursor_row_id = excluded.cursor_row_id, last_flush_at = excluded.last_flush_at, last_error = NULL, updated_at = datetime('now')",
    )
    .bind(LONG_TERM_PROJECTION_CONSUMER)
    .bind(next_cursor)
    .execute(&mut *tx)
    .await?;
    let statistics_start_date = daily
        .values()
        .filter_map(|bucket| bucket.stats_date.as_deref())
        .min()
        .map(str::to_string);
    sqlx::query(
        "UPDATE long_term_stats_state SET status = CASE WHEN ?1 > 0 THEN ?2 ELSE status END, statistics_start_date = CASE WHEN ?3 IS NULL THEN statistics_start_date WHEN statistics_start_date IS NULL OR ?3 < statistics_start_date THEN ?3 ELSE statistics_start_date END, processed_rows = processed_rows + ?1, total_rows = total_rows + ?1, last_error = NULL, updated_at = datetime('now') WHERE id = ?4",
    )
    .bind(event_count as i64)
    .bind(LONG_TERM_STATUS_READY)
    .bind(statistics_start_date)
    .bind(LONG_TERM_STATE_ID)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let mut runtime = runtime.lock().await;
    runtime.interval_index = interval_index;
    runtime.loaded_interval_dates = loaded_dates;
    Ok(())
}

async fn ensure_long_term_projection_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_state (
            consumer TEXT PRIMARY KEY,
            cursor_row_id INTEGER NOT NULL DEFAULT 0,
            last_flush_at TEXT,
            last_error TEXT,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection state table")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_dirty_buckets (
            bucket_date TEXT PRIMARY KEY,
            repair_reason TEXT NOT NULL,
            queued_at TEXT NOT NULL DEFAULT (datetime('now')),
            next_attempt_at TEXT,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection dirty bucket table")?;
    let next_attempt_migration = sqlx::query(
        "ALTER TABLE long_term_projection_dirty_buckets ADD COLUMN next_attempt_at TEXT",
    )
    .execute(pool)
    .await;
    if let Err(error) = next_attempt_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_bucket_state (
            bucket_date TEXT PRIMARY KEY,
            interval_baseline_ready INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection bucket state table")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_intervals (
            bucket_kind TEXT NOT NULL,
            bucket_date TEXT NOT NULL,
            bucket_key TEXT NOT NULL,
            dimension TEXT NOT NULL,
            series_key TEXT NOT NULL,
            invocation_row_id INTEGER NOT NULL,
            interval_start_ms INTEGER NOT NULL,
            interval_end_ms INTEGER NOT NULL,
            PRIMARY KEY (
                bucket_kind,
                bucket_key,
                dimension,
                series_key,
                invocation_row_id,
                interval_start_ms,
                interval_end_ms
            )
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection interval table")?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_long_term_projection_intervals_date ON long_term_projection_intervals (bucket_date, bucket_kind)",
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection interval date index")?;
    Ok(())
}

async fn load_long_term_rollups(
    pool: &Pool<Sqlite>,
    range: LongTermRange,
    dimension: &str,
    start_date: &str,
    end_date: &str,
) -> Result<Vec<LongTermRollupRow>> {
    let table = "long_term_usage_daily";
    let sql = format!(
        "SELECT MAX(d.stats_date) AS bucket_or_date, d.dimension, d.series_key, COALESCE((SELECT account.display_name FROM pool_upstream_accounts account WHERE d.dimension = 'upstream' AND d.series_key LIKE 'account:%' AND account.id = CAST(substr(d.series_key, 9) AS INTEGER) LIMIT 1), (SELECT latest.display_name FROM {table} latest WHERE latest.dimension = ?1 AND latest.series_key = d.series_key AND latest.reasoning_effort = d.reasoning_effort AND latest.stats_date BETWEEN ?2 AND ?3 ORDER BY latest.stats_date DESC LIMIT 1)) AS display_name, d.reasoning_effort, SUM(d.calls) AS calls, SUM(d.token_total) AS token_total, SUM(d.token_samples) AS token_samples, SUM(d.cost_total) AS cost_total, SUM(d.cost_samples) AS cost_samples, SUM(d.usage_time_ms) AS usage_time_ms, SUM(d.usage_time_samples) AS usage_time_samples, SUM(d.wall_time_ms) AS wall_time_ms, SUM(d.wall_time_samples) AS wall_time_samples, SUM(d.output_tokens_total) AS output_tokens_total, SUM(d.stream_duration_ms) AS stream_duration_ms, SUM(d.output_speed_samples) AS output_speed_samples, SUM(d.first_byte_sum_ms) AS first_byte_sum_ms, SUM(d.first_byte_samples) AS first_byte_samples, SUM(d.response_sum_ms) AS response_sum_ms, SUM(d.response_samples) AS response_samples FROM {table} d WHERE d.dimension = ?1 AND d.stats_date BETWEEN ?2 AND ?3 GROUP BY d.series_key, d.reasoning_effort"
    );
    let _ = range;
    Ok(sqlx::query_as::<_, LongTermRollupRow>(&sql)
        .bind(dimension)
        .bind(start_date)
        .bind(end_date)
        .fetch_all(pool)
        .await?)
}

async fn load_long_term_daily_rows(
    pool: &Pool<Sqlite>,
    dimension: &str,
    series_key: Option<&str>,
    start_date: &str,
    end_date: &str,
) -> Result<Vec<LongTermRollupRow>> {
    let mut sql = String::from(
        "SELECT stats_date AS bucket_or_date, dimension, series_key, display_name, reasoning_effort, calls, token_total, token_samples, cost_total, cost_samples, usage_time_ms, usage_time_samples, wall_time_ms, wall_time_samples, output_tokens_total, stream_duration_ms, output_speed_samples, first_byte_sum_ms, first_byte_samples, response_sum_ms, response_samples FROM long_term_usage_daily WHERE dimension = ?1 AND stats_date BETWEEN ?2 AND ?3",
    );
    if series_key.is_some() {
        sql.push_str(" AND series_key = ?4");
    }
    sql.push_str(" ORDER BY stats_date ASC, series_key ASC");
    let mut query = sqlx::query_as::<_, LongTermRollupRow>(&sql)
        .bind(dimension)
        .bind(start_date)
        .bind(end_date);
    if let Some(series_key) = series_key {
        query = query.bind(series_key);
    }
    Ok(query.fetch_all(pool).await?)
}

pub(crate) async fn fetch_long_term_overview(
    State(state): State<Arc<AppState>>,
    Query(query): Query<LongTermRangeQuery>,
) -> Result<Json<LongTermStatsOverviewResponse>, (StatusCode, String)> {
    let range = LongTermRange::parse(query.range.as_deref())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "invalid range".to_string()))?;
    let state_row = load_long_term_state(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let status = normalize_long_term_response_status(&state_row.status);
    if status != LONG_TERM_STATUS_READY {
        return Ok(Json(LongTermStatsOverviewResponse {
            status,
            statistics_start_date: state_row.statistics_start_date,
            processed_rows: state_row.processed_rows,
            total_rows: state_row.total_rows,
            timezone: LONG_TERM_TIMEZONE,
            range: range.as_str().to_string(),
            global: LongTermMetrics::default(),
            daily: Vec::new(),
            models: Vec::new(),
            upstreams: Vec::new(),
        }));
    }
    let (start_date, end_date) =
        long_term_date_window(range, state_row.statistics_start_date.as_deref());
    let daily_rows =
        load_long_term_daily_rows(&state.pool, "overall", None, &start_date, &end_date)
            .await
            .map_err(internal_error_tuple)?;
    let global = aggregate_rollup_rows(&daily_rows);
    let daily = build_daily_points(&daily_rows, &start_date, &end_date);
    let models = build_series_summaries(
        &load_long_term_rollups(&state.pool, range, "model", &start_date, &end_date)
            .await
            .map_err(internal_error_tuple)?,
    );
    let upstreams = build_series_summaries(
        &load_long_term_rollups(&state.pool, range, "upstream", &start_date, &end_date)
            .await
            .map_err(internal_error_tuple)?,
    );
    Ok(Json(LongTermStatsOverviewResponse {
        status,
        statistics_start_date: state_row.statistics_start_date,
        processed_rows: state_row.processed_rows,
        total_rows: state_row.total_rows,
        timezone: LONG_TERM_TIMEZONE,
        range: range.as_str().to_string(),
        global,
        daily,
        models,
        upstreams,
    }))
}

pub(crate) async fn fetch_long_term_series(
    State(state): State<Arc<AppState>>,
    OriginalUri(original_uri): OriginalUri,
) -> Result<Json<LongTermStatsSeriesResponse>, (StatusCode, String)> {
    let query = parse_long_term_series_query(&original_uri);
    let range = LongTermRange::parse(query.range.as_deref())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "invalid range".to_string()))?;
    let dimension = query.dimension.as_deref().unwrap_or_default();
    if !matches!(dimension, "model" | "upstream") {
        return Err((StatusCode::BAD_REQUEST, "invalid dimension".to_string()));
    }
    let keys = query
        .key
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if keys.is_empty() || keys.len() > 8 {
        return Err((
            StatusCode::BAD_REQUEST,
            "key must contain 1 to 8 series keys".to_string(),
        ));
    }
    let state_row = load_long_term_state(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let status = normalize_long_term_response_status(&state_row.status);
    if status != LONG_TERM_STATUS_READY {
        return Ok(Json(LongTermStatsSeriesResponse {
            status,
            statistics_start_date: state_row.statistics_start_date,
            processed_rows: state_row.processed_rows,
            total_rows: state_row.total_rows,
            timezone: LONG_TERM_TIMEZONE,
            range: range.as_str().to_string(),
            dimension: dimension.to_string(),
            series: Vec::new(),
        }));
    }
    let (start_date, end_date) =
        long_term_date_window(range, state_row.statistics_start_date.as_deref());
    let available = load_long_term_rollups(&state.pool, range, dimension, &start_date, &end_date)
        .await
        .map_err(internal_error_tuple)?;
    let available_keys = available
        .iter()
        .map(|row| row.series_key.as_str())
        .collect::<HashSet<_>>();
    if keys
        .iter()
        .any(|key| !available_keys.contains(key.as_str()))
    {
        return Err((StatusCode::BAD_REQUEST, "unknown series key".to_string()));
    }
    let mut series = Vec::with_capacity(keys.len());
    for key in keys {
        let matching =
            load_long_term_daily_rows(&state.pool, dimension, Some(&key), &start_date, &end_date)
                .await
                .map_err(internal_error_tuple)?;
        let summary = available
            .iter()
            .find(|row| row.series_key.as_str() == key.as_str());
        series.push(LongTermSeries {
            series_key: key,
            display_name: summary
                .map(|row| row.display_name.clone())
                .unwrap_or_default(),
            reasoning_effort: summary.and_then(|row| {
                (!row.reasoning_effort.is_empty()).then_some(row.reasoning_effort.clone())
            }),
            points: build_daily_points(&matching, &start_date, &end_date),
        });
    }
    Ok(Json(LongTermStatsSeriesResponse {
        status,
        statistics_start_date: state_row.statistics_start_date,
        processed_rows: state_row.processed_rows,
        total_rows: state_row.total_rows,
        timezone: LONG_TERM_TIMEZONE,
        range: range.as_str().to_string(),
        dimension: dimension.to_string(),
        series,
    }))
}

async fn load_long_term_state(pool: &Pool<Sqlite>) -> Result<LongTermStateRow> {
    Ok(sqlx::query_as::<_, LongTermStateRow>(
        "SELECT status, statistics_start_date, integrity_source_start_date, processed_rows, total_rows, last_error FROM long_term_stats_state WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .fetch_one(pool)
    .await?)
}

fn normalize_long_term_response_status(status: &str) -> String {
    match status {
        LONG_TERM_STATUS_READY | LONG_TERM_STATUS_EMPTY | LONG_TERM_STATUS_PREPARING => {
            status.to_string()
        }
        LONG_TERM_STATUS_DISABLED => LONG_TERM_STATUS_PREPARING.to_string(),
        LONG_TERM_STATUS_RUNNING => LONG_TERM_STATUS_PREPARING.to_string(),
        _ => LONG_TERM_STATUS_ERROR.to_string(),
    }
}

fn long_term_date_window(range: LongTermRange, start_date: Option<&str>) -> (String, String) {
    let today = Utc::now().with_timezone(&Shanghai).date_naive();
    let requested_start = today - ChronoDuration::days(range.days() - 1);
    let effective_start = start_date
        .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
        .map(|value| value.max(requested_start))
        .unwrap_or(requested_start);
    (effective_start.to_string(), today.to_string())
}

fn build_series_summaries(rows: &[LongTermRollupRow]) -> Vec<LongTermSeriesSummary> {
    let mut summaries = rows
        .iter()
        .map(|row| LongTermSeriesSummary {
            series_key: row.series_key.clone(),
            display_name: row.display_name.clone(),
            reasoning_effort: (!row.reasoning_effort.is_empty())
                .then_some(row.reasoning_effort.clone()),
            metrics: LongTermMetrics::from_rollup(row),
        })
        .collect::<Vec<_>>();
    summaries.sort_by(|left, right| {
        right
            .metrics
            .tokens
            .unwrap_or_default()
            .cmp(&left.metrics.tokens.unwrap_or_default())
            .then_with(|| left.display_name.cmp(&right.display_name))
    });
    summaries
}

fn aggregate_rollup_rows(rows: &[LongTermRollupRow]) -> LongTermMetrics {
    let mut acc = LongTermAccumulator::default();
    let mut wall_time_ms = 0.0;
    let mut wall_samples = 0_i64;
    for row in rows {
        acc.calls += row.calls;
        acc.token_total += row.token_total;
        acc.token_samples += row.token_samples;
        acc.cost_total += row.cost_total;
        acc.cost_samples += row.cost_samples;
        acc.usage_time_ms += row.usage_time_ms;
        acc.usage_time_samples += row.usage_time_samples;
        acc.output_tokens_total += row.output_tokens_total;
        acc.stream_duration_ms += row.stream_duration_ms;
        acc.output_speed_samples += row.output_speed_samples;
        acc.first_byte_sum_ms += row.first_byte_sum_ms;
        acc.first_byte_samples += row.first_byte_samples;
        acc.response_sum_ms += row.response_sum_ms;
        acc.response_samples += row.response_samples;
        wall_time_ms += row.wall_time_ms;
        wall_samples += row.wall_time_samples;
    }
    let mut metrics = LongTermMetrics::from_accumulator(&acc);
    metrics.wall_time_ms = (wall_samples > 0).then_some(wall_time_ms);
    metrics.wall_time_samples = wall_samples;
    metrics
}

fn build_daily_points(
    rows: &[LongTermRollupRow],
    start_date: &str,
    end_date: &str,
) -> Vec<LongTermDailyPoint> {
    let mut by_date: HashMap<&str, (LongTermAccumulator, f64, i64)> = HashMap::new();
    for row in rows {
        let (acc, wall_time_ms, wall_samples) = by_date
            .entry(row.bucket_or_date.as_str())
            .or_insert_with(|| (LongTermAccumulator::default(), 0.0, 0));
        acc.calls += row.calls;
        acc.token_total += row.token_total;
        acc.token_samples += row.token_samples;
        acc.cost_total += row.cost_total;
        acc.cost_samples += row.cost_samples;
        acc.usage_time_ms += row.usage_time_ms;
        acc.usage_time_samples += row.usage_time_samples;
        acc.output_tokens_total += row.output_tokens_total;
        acc.stream_duration_ms += row.stream_duration_ms;
        acc.output_speed_samples += row.output_speed_samples;
        acc.first_byte_sum_ms += row.first_byte_sum_ms;
        acc.first_byte_samples += row.first_byte_samples;
        acc.response_sum_ms += row.response_sum_ms;
        acc.response_samples += row.response_samples;
        *wall_time_ms += row.wall_time_ms;
        *wall_samples += row.wall_time_samples;
    }
    let start = NaiveDate::parse_from_str(start_date, "%Y-%m-%d")
        .unwrap_or_else(|_| Utc::now().with_timezone(&Shanghai).date_naive());
    let end = NaiveDate::parse_from_str(end_date, "%Y-%m-%d").unwrap_or(start);
    let mut points = Vec::new();
    let mut date = start;
    while date <= end {
        let date_string = date.to_string();
        let metrics =
            if let Some((acc, wall_time_ms, wall_samples)) = by_date.get(date_string.as_str()) {
                let mut metrics = LongTermMetrics::from_accumulator(acc);
                metrics.wall_time_ms = (*wall_samples > 0).then_some(*wall_time_ms);
                metrics.wall_time_samples = *wall_samples;
                metrics
            } else {
                LongTermMetrics::default()
            };
        points.push(LongTermDailyPoint {
            date: date_string,
            metrics,
        });
        let Some(next) = date.succ_opt() else { break };
        date = next;
    }
    points
}

fn normalize_long_term_model(row: &LongTermInvocationRow) -> String {
    for candidate in [
        row.response_model.as_deref(),
        row.model.as_deref(),
        row.request_model.as_deref(),
    ] {
        if let Some(value) = candidate.map(str::trim).filter(|value| !value.is_empty()) {
            return value.to_string();
        }
    }
    "未知模型".to_string()
}

fn normalize_long_term_reasoning(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("未指定")
        .to_string()
}

fn long_term_model_series_key(model: &str, reasoning: &str) -> String {
    let payload =
        serde_json::to_vec(&[model, reasoning]).expect("model series key payload is serializable");
    format!(
        "model:v2:{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload)
    )
}

fn normalize_long_term_upstream(row: &LongTermInvocationRow) -> (String, String) {
    if row
        .upstream_account_kind
        .as_deref()
        .is_some_and(|kind| kind.eq_ignore_ascii_case(UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX))
        && let Some(id) = row.upstream_account_id
    {
        return (
            format!("account:{id}"),
            row.upstream_account_name
                .clone()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| format!("账号 {id}")),
        );
    }
    (
        LONG_TERM_OTHER_KEY.to_string(),
        LONG_TERM_OTHER_NAME.to_string(),
    )
}

fn is_success_status(status: Option<&str>, error_message: Option<&str>) -> bool {
    invocation_status_is_success_like(status, error_message)
        || matches!(
            status
                .map(str::trim)
                .unwrap_or_default()
                .to_ascii_lowercase()
                .as_str(),
            "succeeded" | "ok"
        )
}

fn parse_long_term_timestamp_ms(raw: &str) -> Option<i64> {
    if let Ok(value) = DateTime::parse_from_rfc3339(raw) {
        return Some(value.timestamp_millis());
    }
    let naive = NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f")
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S"))
        .ok()?;
    Shanghai
        .from_local_datetime(&naive)
        .single()
        .map(|value| value.timestamp_millis())
}

fn union_interval_duration(intervals: &[(i64, i64)]) -> i64 {
    if intervals.is_empty() {
        return 0;
    }
    let mut sorted = intervals.to_vec();
    sorted.sort_unstable_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    let mut total = 0_i64;
    let mut current = sorted[0];
    for next in sorted.into_iter().skip(1) {
        if next.0 <= current.1 {
            current.1 = current.1.max(next.1);
        } else {
            total += current.1.saturating_sub(current.0);
            current = next;
        }
    }
    total + current.1.saturating_sub(current.0)
}

fn internal_error_tuple(error: anyhow::Error) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn create_long_term_test_invocations(pool: &Pool<Sqlite>) {
        sqlx::query(
            r#"
            CREATE TABLE codex_invocations (
                id INTEGER PRIMARY KEY,
                invoke_id TEXT,
                occurred_at TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'canonical',
                status TEXT,
                detail_level TEXT NOT NULL DEFAULT 'full',
                model TEXT,
                input_tokens INTEGER,
                output_tokens INTEGER,
                cache_input_tokens INTEGER,
                total_tokens INTEGER,
                cost REAL,
                cost_input REAL,
                cost_cache_write REAL,
                cost_cache_read REAL,
                cost_output REAL,
                cost_reasoning REAL,
                error_message TEXT,
                failure_kind TEXT,
                failure_class TEXT,
                is_actionable INTEGER NOT NULL DEFAULT 0,
                payload TEXT,
                t_total_ms REAL,
                t_req_read_ms REAL,
                t_req_parse_ms REAL,
                t_upstream_connect_ms REAL,
                t_upstream_ttfb_ms REAL,
                first_token_ms REAL,
                t_upstream_stream_ms REAL,
                t_resp_parse_ms REAL,
                t_persist_ms REAL
            )
            "#,
        )
        .execute(pool)
        .await
        .expect("invocation schema");
    }

    async fn create_long_term_integrity_oracle(pool: &Pool<Sqlite>) {
        sqlx::query(
            r#"
            CREATE TABLE invocation_rollup_hourly (
                bucket_start_epoch INTEGER NOT NULL,
                source TEXT NOT NULL,
                total_count INTEGER NOT NULL,
                success_count INTEGER NOT NULL DEFAULT 0,
                failure_count INTEGER NOT NULL DEFAULT 0,
                terminal_count INTEGER NOT NULL,
                terminal_tokens INTEGER NOT NULL,
                terminal_cost REAL NOT NULL,
                terminal_proof_complete INTEGER NOT NULL DEFAULT 1,
                total_tokens INTEGER NOT NULL,
                cache_input_tokens INTEGER NOT NULL DEFAULT 0,
                total_cost REAL NOT NULL,
                non_success_cost REAL NOT NULL DEFAULT 0,
                total_latency_sample_count INTEGER NOT NULL DEFAULT 0,
                total_latency_sum_ms REAL NOT NULL DEFAULT 0,
                first_byte_sample_count INTEGER NOT NULL DEFAULT 0,
                first_byte_sum_ms REAL NOT NULL DEFAULT 0,
                first_byte_max_ms REAL NOT NULL DEFAULT 0,
                first_byte_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
                first_response_byte_total_sample_count INTEGER NOT NULL DEFAULT 0,
                first_response_byte_total_sum_ms REAL NOT NULL DEFAULT 0,
                first_response_byte_total_max_ms REAL NOT NULL DEFAULT 0,
                first_response_byte_total_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
                first_token_sample_count INTEGER NOT NULL DEFAULT 0,
                first_token_sum_ms REAL NOT NULL DEFAULT 0,
                first_token_max_ms REAL NOT NULL DEFAULT 0,
                first_token_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (bucket_start_epoch, source)
            )
            "#,
        )
        .execute(pool)
        .await
        .expect("hourly integrity oracle schema");
    }

    async fn long_term_file_backed_pool(prefix: &str) -> (Pool<Sqlite>, String, PathBuf) {
        let db_path = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-{prefix}-{}-{}.sqlite",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        fs::File::create(&db_path).expect("create sqlite test database");
        let db_url = format!("sqlite://{}", db_path.to_string_lossy());
        let options = db_url
            .parse::<SqliteConnectOptions>()
            .expect("parse sqlite test url")
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_millis(50));
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .expect("connect sqlite test pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        (pool, db_url, db_path)
    }

    async fn cleanup_long_term_file_backed_pool(pool: Pool<Sqlite>, db_path: PathBuf) {
        pool.close().await;
        for suffix in ["", "-shm", "-wal"] {
            let path = PathBuf::from(format!("{}{}", db_path.display(), suffix));
            let _ = fs::remove_file(path);
        }
    }

    async fn insert_long_term_test_invocation(pool: &Pool<Sqlite>, id: i64, occurred_at: String) {
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, model, payload, total_tokens, output_tokens, cost) VALUES (?1, ?2, ?3, 'success', 'gpt-5', '{}', 100, 40, 0.1)",
        )
        .bind(id)
        .bind(format!("invoke-{id}"))
        .bind(occurred_at)
        .execute(pool)
        .await
        .expect("source invocation");
    }

    async fn seed_long_term_integrity_case(
        pool: &Pool<Sqlite>,
        date: NaiveDate,
        source_rows: i64,
    ) -> (i64, i64) {
        let (day_start_epoch, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
        for offset in 0..source_rows {
            let hour = 10 + offset;
            sqlx::query(
                "INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, status, model, payload, total_tokens, output_tokens, cost) VALUES (?1, ?2, ?3, ?4, 'success', 'gpt-5', '{\"reasoningEffort\":\"high\"}', 100, 40, 0.1)",
            )
            .bind(offset + 1)
            .bind(format!("invoke-{}", offset + 1))
            .bind(format!("{date}T{:02}:00:00+08:00", hour))
            .bind(format!("canonical-{}", offset + 1))
            .execute(pool)
            .await
            .expect("source invocation");
        }
        for offset in 0..2_i64 {
            let hour_epoch = day_start_epoch + (10 + offset) * 60 * 60;
            sqlx::query(
                "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, terminal_count, terminal_tokens, terminal_cost, total_tokens, total_cost) VALUES (?1, ?2, 1, 1, 100, 0.1, 100, 0.1)",
            )
            .bind(hour_epoch)
            .bind(format!("canonical-{}", offset + 1))
            .execute(pool)
            .await
            .expect("canonical hourly rollup");
        }
        sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(date.to_string())
        .execute(pool)
        .await
        .expect("corrupt daily rollup");
        sqlx::query(
            "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(day_start_epoch + 10 * 60 * 60)
        .execute(pool)
        .await
        .expect("corrupt hourly rollup");
        sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
            .bind(LONG_TERM_STATUS_READY)
            .bind(LONG_TERM_STATE_ID)
            .execute(pool)
            .await
            .expect("ready state");
        (
            day_start_epoch + 10 * 60 * 60,
            day_start_epoch + 11 * 60 * 60,
        )
    }

    #[tokio::test]
    async fn legacy_source_timing_queries_accept_missing_optional_columns() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        sqlx::query("CREATE TABLE codex_invocations (occurred_at TEXT NOT NULL)")
            .execute(&pool)
            .await
            .expect("legacy invocation schema without optional columns");
        sqlx::query("INSERT INTO codex_invocations (occurred_at) VALUES ('2025-01-01 10:00:00')")
            .execute(&pool)
            .await
            .expect("legacy invocation row without optional columns");

        let archive_columns = load_archive_table_columns(&pool, "codex_invocations")
            .await
            .expect("legacy archive columns");
        let archive_rows = sqlx::query_as::<_, LongTermSourceTimingRow>(
            &long_term_source_timing_archive_query(&archive_columns),
        )
        .fetch_all(&pool)
        .await
        .expect("source timing query accepts absent optional archive columns");
        assert_eq!(archive_rows.len(), 1);
        assert_eq!(archive_rows[0].invoke_id, None);
        assert_eq!(archive_rows[0].t_total_ms, None);

        sqlx::query("DROP TABLE codex_invocations")
            .execute(&pool)
            .await
            .expect("replace legacy invocation schema");
        sqlx::query(
            "CREATE TABLE codex_invocations (invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .expect("legacy invocation schema without timing column");
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at) VALUES ('legacy-invoke', '2025-01-01 10:00:00')",
        )
        .execute(&pool)
        .await
        .expect("legacy invocation row without timing column");

        let archive_columns = load_archive_table_columns(&pool, "codex_invocations")
            .await
            .expect("legacy archive columns with invoke id");
        let pairs = HashSet::from([(
            "legacy-invoke".to_string(),
            "2025-01-01 10:00:00".to_string(),
        )]);
        let matched_rows = load_long_term_source_timing_rows_for_pairs(
            &pool,
            &pairs,
            &long_term_legacy_column_expr(&archive_columns, "t_total_ms"),
        )
        .await
        .expect("matched source timing query accepts absent timing column");
        assert_eq!(matched_rows.len(), 1);
        assert_eq!(matched_rows[0].invoke_id.as_deref(), Some("legacy-invoke"));
        assert_eq!(matched_rows[0].t_total_ms, None);
    }

    #[tokio::test]
    async fn queued_repair_attempt_scan_covers_the_replayed_archive_date() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("full schema");
        let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
        let month_key = date.format("%Y-%m").to_string();
        let occurred_at = format!("{date} 10:00:00");
        let archive_db_path = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-repair-attempt-source-{}-{}.sqlite",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        let archive_path = archive_db_path.with_extension("sqlite.gz");
        fs::File::create(&archive_db_path).expect("create attempt archive database");
        let archive_options = format!("sqlite://{}", archive_db_path.to_string_lossy())
            .parse::<SqliteConnectOptions>()
            .expect("parse attempt archive URL")
            .create_if_missing(true);
        let archive_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(archive_options)
            .await
            .expect("open attempt archive database");
        sqlx::query(
            "CREATE TABLE pool_upstream_request_attempts (id INTEGER PRIMARY KEY, invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL, upstream_account_id INTEGER)",
        )
        .execute(&archive_pool)
        .await
        .expect("create attempt archive schema");
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, upstream_account_id) VALUES (1, 'repair-invoke', ?1, 42)",
        )
        .bind(&occurred_at)
        .execute(&archive_pool)
        .await
        .expect("insert archived attempt mapping");
        archive_pool.close().await;
        crate::maintenance::deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
            .expect("compress attempt archive");
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                dataset,
                month_key,
                file_path,
                sha256,
                row_count,
                status,
                coverage_start_at,
                coverage_end_at,
                created_at
            )
            VALUES ('pool_upstream_request_attempts', ?1, ?2, 'repair-attempt-sha', 1, 'completed', ?3, ?3, datetime('now'))
            "#,
        )
        .bind(month_key)
        .bind(archive_path.to_string_lossy().to_string())
        .bind(&occurred_at)
        .execute(&pool)
        .await
        .expect("record attempt archive manifest");

        let previous_date = date.pred_opt().expect("previous date");
        let next_date = date.succ_opt().expect("next date");
        let (outside_range, _) =
            load_long_term_archive_attempt_accounts(&pool, Some((next_date, next_date)))
                .await
                .expect("scan outside queued repair date");
        let (queued_repair_accounts, _) =
            load_long_term_archive_attempt_accounts(&pool, Some((date, date)))
                .await
                .expect("scan queued repair date");
        let (cross_midnight_repair_accounts, _) =
            load_long_term_archive_attempt_accounts(&pool, Some((previous_date, date)))
                .await
                .expect("scan final rebuild range including the preceding date");

        assert!(outside_range.is_empty());
        assert_eq!(
            queued_repair_accounts.get(&("repair-invoke".to_string(), occurred_at.clone())),
            Some(&42)
        );
        assert_eq!(
            cross_midnight_repair_accounts.get(&("repair-invoke".to_string(), occurred_at)),
            Some(&42)
        );

        let _ = fs::remove_file(&archive_db_path);
        let _ = fs::remove_file(&archive_path);
    }

    #[tokio::test]
    async fn attempt_account_scan_rejects_missing_completed_archive() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("full schema");
        let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
        let occurred_at = format!("{date} 10:00:00");
        let missing_path = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-missing-attempt-source-{}-{}.sqlite.gz",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                dataset, month_key, file_path, sha256, row_count, status,
                coverage_start_at, coverage_end_at, created_at
            )
            VALUES ('pool_upstream_request_attempts', ?1, ?2, 'missing-attempt-sha', 1,
                'completed', ?3, ?3, datetime('now'))
            "#,
        )
        .bind(date.format("%Y-%m").to_string())
        .bind(missing_path.to_string_lossy().to_string())
        .bind(&occurred_at)
        .execute(&pool)
        .await
        .expect("record missing attempt archive manifest");

        let error = load_long_term_archive_attempt_accounts(&pool, Some((date, date)))
            .await
            .expect_err("a missing completed archive cannot be treated as an empty mapping");
        assert!(error.to_string().contains("attempt archive is unavailable"));
    }

    #[tokio::test]
    async fn attempt_account_scan_rejects_damaged_completed_archive() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("full schema");
        let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
        let occurred_at = format!("{date} 10:00:00");
        let damaged_path = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-damaged-attempt-source-{}-{}.sqlite.gz",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        fs::write(&damaged_path, b"not a gzip archive").expect("write damaged archive");
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                dataset, month_key, file_path, sha256, row_count, status,
                coverage_start_at, coverage_end_at, created_at
            )
            VALUES ('pool_upstream_request_attempts', ?1, ?2, 'damaged-attempt-sha', 1,
                'completed', ?3, ?3, datetime('now'))
            "#,
        )
        .bind(date.format("%Y-%m").to_string())
        .bind(damaged_path.to_string_lossy().to_string())
        .bind(&occurred_at)
        .execute(&pool)
        .await
        .expect("record damaged attempt archive manifest");

        let error = load_long_term_archive_attempt_accounts(&pool, Some((date, date)))
            .await
            .expect_err("a damaged completed archive cannot be treated as an empty mapping");
        assert!(
            error
                .to_string()
                .contains(LONG_TERM_ATTEMPT_ARCHIVE_UNAVAILABLE_ERROR)
        );
        let _ = fs::remove_file(damaged_path);
    }

    #[tokio::test]
    async fn refresh_hides_ready_stats_when_attempt_attribution_source_is_unavailable() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("full schema");
        let date = Utc::now().with_timezone(&Shanghai).date_naive();
        let occurred_at = format!("{date}T10:00:00+08:00");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, model, payload, raw_response, total_tokens, output_tokens, cost, created_at) VALUES (1, 'missing-attribution-source', ?1, 'success', 'gpt-5', '{}', '{}', 100, 40, 0.1, datetime('now'))",
        )
        .bind(&occurred_at)
        .execute(&pool)
        .await
        .expect("insert live invocation requiring account attribution");
        sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("seed visible durable long-term data");
        sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
            .bind(LONG_TERM_STATUS_READY)
            .bind(LONG_TERM_STATE_ID)
            .execute(&pool)
            .await
            .expect("seed ready state");
        let missing_path = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-missing-ready-attempt-source-{}-{}.sqlite.gz",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                dataset, month_key, file_path, sha256, row_count, status,
                coverage_start_at, coverage_end_at, created_at
            )
            VALUES ('pool_upstream_request_attempts', ?1, ?2, 'missing-ready-attempt-sha',
                1, 'completed', ?3, ?3, datetime('now'))
            "#,
        )
        .bind(date.format("%Y-%m").to_string())
        .bind(missing_path.to_string_lossy().to_string())
        .bind(&occurred_at)
        .execute(&pool)
        .await
        .expect("record missing attribution source");

        let error = refresh_long_term_stats(&pool, 400)
            .await
            .expect_err("missing attribution source must block a ready refresh");
        assert!(
            error
                .to_string()
                .contains(LONG_TERM_ATTEMPT_ARCHIVE_UNAVAILABLE_ERROR)
        );
        let state: (String, Option<String>) =
            sqlx::query_as("SELECT status, last_error FROM long_term_stats_state WHERE id = ?1")
                .bind(LONG_TERM_STATE_ID)
                .fetch_one(&pool)
                .await
                .expect("load blocked long-term state");
        assert_eq!(state.0, LONG_TERM_STATUS_ERROR);
        assert!(
            state.1.as_deref().is_some_and(
                |message| message.contains(LONG_TERM_ATTEMPT_ARCHIVE_UNAVAILABLE_ERROR)
            )
        );
        let calls = sqlx::query_scalar::<_, i64>(
            "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("load preserved durable data");
        assert_eq!(calls, 1);
    }

    #[test]
    fn matched_attempt_source_rows_require_parseable_effective_dates() {
        let pair = (
            "matched-attempt-source".to_string(),
            "invalid-timestamp".to_string(),
        );
        let mut unmatched_pairs = HashSet::from([pair.clone()]);
        let mut latest_effective_date = None;

        let error = record_long_term_matched_attempt_source_rows(
            &mut unmatched_pairs,
            &mut latest_effective_date,
            vec![LongTermSourceTimingRow {
                invoke_id: Some(pair.0.clone()),
                occurred_at: pair.1.clone(),
                t_total_ms: None,
            }],
        )
        .expect_err("a matched source with an invalid timestamp cannot prove an archive boundary");

        assert!(error.to_string().contains("unparseable timestamp"));
        assert_eq!(unmatched_pairs, HashSet::from([pair]));
        assert_eq!(latest_effective_date, None);
    }

    #[test]
    fn range_defaults_to_seven_days_and_rejects_unknown_values() {
        assert_eq!(LongTermRange::parse(None), Some(LongTermRange::Seven));
        assert_eq!(
            LongTermRange::parse(Some("365d")),
            Some(LongTermRange::ThreeSixtyFive)
        );
        assert_eq!(LongTermRange::parse(Some("1d")), None);
    }

    #[test]
    fn refresh_keeps_error_visible_while_an_integrity_repair_is_pending() {
        assert_eq!(
            long_term_refresh_start_state(true, true),
            (LONG_TERM_STATUS_ERROR, false)
        );
        assert_eq!(
            long_term_refresh_start_state(true, false),
            (LONG_TERM_STATUS_READY, true)
        );
        assert_eq!(
            long_term_refresh_start_state(false, false),
            (LONG_TERM_STATUS_RUNNING, true)
        );
        assert_eq!(
            long_term_refresh_start_state(false, true),
            (LONG_TERM_STATUS_ERROR, false)
        );
    }

    #[test]
    fn reconstructable_start_uses_the_persisted_source_boundary() {
        let retention_start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("fixed retention start");
        let archived_through = NaiveDate::from_ymd_opt(2026, 7, 23).expect("fixed archive end");

        assert_eq!(
            long_term_source_safe_start_after_effective_date(archived_through),
            NaiveDate::from_ymd_opt(2026, 7, 24).expect("exact successor safe start")
        );
        assert_eq!(
            long_term_reconstructable_start(retention_start, None, Some("2026-07-25")),
            NaiveDate::from_ymd_opt(2026, 7, 25).expect("persisted safe start")
        );
    }

    #[tokio::test]
    async fn unreadable_source_without_a_coverage_start_blocks_the_full_retention_window() {
        let retention_start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("fixed retention start");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("full schema");
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                dataset,
                month_key,
                file_path,
                sha256,
                row_count,
                status,
                coverage_end_at,
                created_at
            )
            VALUES ('codex_invocations', '2026-07', '/tmp/missing-archive.sqlite.gz', 'missing-sha', 1, 'completed', '2026-07-23 23:59:59', datetime('now'))
            "#,
        )
        .execute(&pool)
        .await
        .expect("missing archive manifest without a coverage start");
        let archive_path = load_completed_invocation_archive_paths(&pool)
            .await
            .expect("load completed archive manifest")
            .into_iter()
            .next()
            .expect("one archive manifest");

        assert_eq!(
            long_term_unreadable_source_start(&archive_path, retention_start),
            retention_start,
            "an end timestamp cannot prove that an unreadable archive has no earlier rows"
        );
    }

    #[tokio::test]
    async fn backfill_start_preserves_error_for_a_pending_integrity_repair() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES ('2026-07-23', 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .execute(&pool)
        .await
        .expect("durable daily rollup");
        sqlx::query(
            "INSERT INTO long_term_stats_repair_queue (stats_date, expected_calls, expected_token_total, expected_cost_total, observed_calls, observed_token_total, observed_cost_total, last_error) VALUES ('2026-07-23', 2, 200, 0.2, 1, 100, 0.1, 'integrity mismatch')",
        )
        .execute(&pool)
        .await
        .expect("pending repair queue");
        sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
            .bind(LONG_TERM_STATUS_ERROR)
            .bind(LONG_TERM_STATE_ID)
            .execute(&pool)
            .await
            .expect("integrity error state");

        mark_long_term_stats_backfill_preparing(&pool)
            .await
            .expect("preserve error state");

        let status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("long-term state");
        assert_eq!(status, LONG_TERM_STATUS_ERROR);
    }

    #[tokio::test]
    async fn integrity_source_boundary_waits_for_contiguous_archive_retirement() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("full schema");
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id,
                dataset,
                month_key,
                file_path,
                sha256,
                row_count,
                status,
                coverage_start_at,
                created_at
            )
            VALUES (2, 'codex_invocations', '2025-01', '/tmp/retained-overlap.sqlite.gz', 'retained-overlap-sha', 1, 'completed', '2025-01-02 00:00:00', datetime('now'))
            "#,
        )
        .execute(&pool)
        .await
        .expect("insert later retained archive coverage");

        let first_safe_start = NaiveDate::from_ymd_opt(2025, 1, 5).expect("fixed date");
        let mut tx = pool.begin().await.expect("start first cleanup transaction");
        advance_long_term_integrity_source_start_tx(tx.as_mut(), 1, first_safe_start)
            .await
            .expect("defer boundary behind retained overlapping source");
        tx.commit().await.expect("commit deferred boundary");

        let state: (Option<String>, Option<String>) = sqlx::query_as(
            "SELECT integrity_source_start_date, integrity_source_pending_start_date FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("load deferred boundary state");
        assert_eq!(state.0, None);
        assert_eq!(state.1.as_deref(), Some("2025-01-05"));

        let mut tx = pool
            .begin()
            .await
            .expect("start contiguous cleanup transaction");
        advance_long_term_integrity_source_start_tx(
            tx.as_mut(),
            2,
            NaiveDate::from_ymd_opt(2025, 1, 4).expect("fixed date"),
        )
        .await
        .expect("commit accumulated safe boundary after final retained source retires");
        tx.commit().await.expect("commit contiguous boundary");

        let state: (Option<String>, Option<String>) = sqlx::query_as(
            "SELECT integrity_source_start_date, integrity_source_pending_start_date FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("load committed contiguous boundary");
        assert_eq!(state.0.as_deref(), Some("2025-01-05"));
        assert_eq!(state.1, None);
    }

    #[test]
    fn series_query_parser_accepts_repeated_keys() {
        let uri: Uri = "/api/stats/long-term/series?range=30d&dimension=model&key=one&key=two"
            .parse()
            .expect("valid series URI");
        let query = parse_long_term_series_query(&uri);
        assert_eq!(query.range.as_deref(), Some("30d"));
        assert_eq!(query.dimension.as_deref(), Some("model"));
        assert_eq!(query.key, ["one", "two"]);
    }

    #[test]
    fn model_series_key_encodes_model_and_reasoning_without_collisions() {
        let left = long_term_model_series_key("a|reasoning:b", "c");
        let right = long_term_model_series_key("a", "b|reasoning:c");
        assert_ne!(left, right);
        assert!(left.starts_with("model:v2:"));
    }

    #[test]
    fn wall_time_union_deduplicates_overlapping_accounts_and_hour_boundaries() {
        assert_eq!(union_interval_duration(&[(0, 10), (5, 15), (20, 25)]), 20);
        assert_eq!(
            union_interval_duration(&[(0, 3_600_000), (0, 3_600_000)]),
            3_600_000
        );
    }

    #[tokio::test]
    async fn integrity_oracle_uses_shanghai_day_boundaries() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        create_long_term_integrity_oracle(&pool).await;
        let date = NaiveDate::from_ymd_opt(2026, 7, 23).expect("fixed date");
        let (start_epoch, end_epoch) = long_term_day_epoch_bounds(date).expect("day bounds");
        for (offset, source) in [
            (-1_i64, "previous-day"),
            (0_i64, "day-start"),
            (23_i64, "day-end"),
            (24_i64, "next-day"),
        ] {
            sqlx::query(
                "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, terminal_count, terminal_tokens, terminal_cost, total_tokens, total_cost) VALUES (?1, ?2, 1, 1, 100, 0.1, 100, 0.1)",
            )
            .bind(start_epoch + offset * 60 * 60)
            .bind(source)
            .execute(&pool)
            .await
            .expect("canonical hourly rollup");
        }

        let oracle = load_long_term_integrity_oracle(&pool, date)
            .await
            .expect("load integrity oracle")
            .expect("oracle rows for requested date");
        assert_eq!(oracle.daily.calls, 2);
        assert_eq!(oracle.daily.token_total, 200);
        assert_eq!(oracle.hourly.len(), 2);
        assert!(oracle.hourly.contains_key(&start_epoch));
        assert!(oracle.hourly.contains_key(&(end_epoch - 60 * 60)));
        assert_eq!(long_term_bucket_date(start_epoch - 1), date.pred_opt());
        assert_eq!(long_term_bucket_date(start_epoch), Some(date));
        assert_eq!(long_term_bucket_date(end_epoch - 1), Some(date));
        assert_eq!(long_term_bucket_date(end_epoch), date.succ_opt());
    }

    #[tokio::test]
    async fn integrity_oracle_represents_an_empty_canonical_day() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        create_long_term_integrity_oracle(&pool).await;
        let date = NaiveDate::from_ymd_opt(2026, 7, 23).expect("fixed date");

        let oracle = load_long_term_integrity_oracle(&pool, date)
            .await
            .expect("load empty integrity oracle")
            .expect("an empty canonical day is still integrity evidence");

        assert_eq!(oracle.date, date);
        assert_eq!(oracle.daily, LongTermIntegrityTotals::default());
        assert!(oracle.hourly.is_empty());
    }

    #[tokio::test]
    async fn integrity_audit_skips_hours_without_terminal_proof() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        create_long_term_integrity_oracle(&pool).await;
        let date = NaiveDate::from_ymd_opt(2026, 7, 23).expect("fixed date");
        let (day_start, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
        sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete, total_tokens, total_cost) VALUES (?1, 'legacy', 1, 1, 100, 0.1, 0, 100, 0.1)",
        )
        .bind(day_start + 10 * 60 * 60)
        .execute(&pool)
        .await
        .expect("untrusted legacy rollup");
        sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("durable daily rollup");

        assert!(
            load_long_term_integrity_oracle(&pool, date)
                .await
                .expect("load integrity oracle")
                .is_none(),
            "a legacy hourly row cannot prove a complete day until terminal totals are backfilled"
        );
        let mismatches = audit_long_term_integrity(&pool, date, date)
            .await
            .expect("audit untrusted hourly rollup");
        assert!(
            mismatches.is_empty(),
            "unknown canonical data must not be treated as a zero-value integrity oracle"
        );
    }

    #[tokio::test]
    async fn terminal_integrity_oracle_ignores_active_invocations() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        create_long_term_test_invocations(&pool).await;
        create_long_term_integrity_oracle(&pool).await;
        let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
        let (day_start, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
        insert_long_term_test_invocation(&pool, 1, format!("{date}T10:00:00+08:00")).await;
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, model, payload, total_tokens, output_tokens, cost) VALUES (2, 'active', ?1, 'running', 'gpt-5', '{}', 100, 40, 0.1)",
        )
        .bind(format!("{date}T10:10:00+08:00"))
        .execute(&pool)
        .await
        .expect("active source invocation");
        sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, terminal_count, terminal_tokens, terminal_cost, total_tokens, total_cost) VALUES (?1, 'canonical', 2, 1, 100, 0.1, 200, 0.2)",
        )
        .bind(day_start + 10 * 60 * 60)
        .execute(&pool)
        .await
        .expect("canonical hourly rollup");

        refresh_long_term_stats(&pool, 400)
            .await
            .expect("refresh with active invocation");

        let calls = sqlx::query_scalar::<_, i64>(
            "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("terminal daily rollup");
        let queued_repairs =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_stats_repair_queue")
                .fetch_one(&pool)
                .await
                .expect("repair queue count");
        let status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("long-term status");
        assert_eq!(calls, 1);
        assert_eq!(queued_repairs, 0);
        assert_eq!(status, LONG_TERM_STATUS_READY);
    }

    #[tokio::test]
    async fn hourly_audit_hides_stats_when_a_previously_trusted_archive_source_disappears() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("full schema");
        let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
        let (day_start, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
        let missing_archive_path = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-missing-trusted-source-{}-{}.sqlite.gz",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        let missing_archive_path = missing_archive_path.to_string_lossy().to_string();

        sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, success_count, failure_count, terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete, total_tokens, total_cost) VALUES (?1, 'trusted', 1, 1, 0, 1, 100, 0.1, 1, 100, 0.1)",
        )
        .bind(day_start + 10 * 60 * 60)
        .execute(&pool)
        .await
        .expect("trusted canonical hourly rollup");
        sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("materialized daily rollup");
        sqlx::query(
            "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(day_start + 10 * 60 * 60)
        .execute(&pool)
        .await
        .expect("materialized hourly rollup");
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                dataset,
                month_key,
                file_path,
                sha256,
                row_count,
                status,
                coverage_start_at,
                coverage_end_at,
                created_at
            )
            VALUES ('codex_invocations', '2026-01', ?1, 'missing-trusted-source-sha', 1, 'completed', ?2, ?2, datetime('now'))
            "#,
        )
        .bind(&missing_archive_path)
        .bind(format!("{date} 10:00:00"))
        .execute(&pool)
        .await
        .expect("missing completed archive manifest");
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'codex_invocations', ?2, 'missing-trusted-source-sha')",
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .bind(&missing_archive_path)
        .execute(&pool)
        .await
        .expect("existing replay marker");
        sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, statistics_start_date = ?2, last_integrity_audit_at = NULL WHERE id = ?3",
        )
        .bind(LONG_TERM_STATUS_READY)
        .bind(date.to_string())
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("make hourly audit due");

        refresh_long_term_stats(&pool, 400)
            .await
            .expect("hourly audit should tolerate a missing replayed archive");

        let (status, last_error) = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT status, last_error FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("long-term state after source loss");
        let proof = sqlx::query_scalar::<_, i64>(
            "SELECT terminal_proof_complete FROM invocation_rollup_hourly WHERE source = 'trusted'",
        )
        .fetch_one(&pool)
        .await
        .expect("revoked terminal proof");
        assert_eq!(status, LONG_TERM_STATUS_ERROR);
        assert_eq!(
            last_error.as_deref(),
            Some(LONG_TERM_TERMINAL_PROOF_UNAVAILABLE_ERROR)
        );
        assert_eq!(proof, 0);
        let replay_markers = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = 'codex_invocations' AND file_path = ?2",
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .bind(&missing_archive_path)
        .fetch_one(&pool)
        .await
        .expect("count replay markers after source loss");
        assert_eq!(
            replay_markers, 0,
            "a missing replayed source must be retried if the archive is restored with the same identity"
        );
    }

    #[tokio::test]
    async fn hourly_audit_repairs_stats_when_complete_sources_disagree_with_canonical() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("full schema");
        let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
        let (day_start, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
        let hour_start = day_start + 10 * 60 * 60;

        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, detail_level, model, payload, raw_response, total_tokens, output_tokens, cost) VALUES ('reconciliation-source', ?1, ?2, 'success', 'full', 'gpt-5', '{}', '{}', 100, 40, 0.1)",
        )
        .bind(format!("{date} 10:00:00"))
        .bind(SOURCE_XY)
        .execute(&pool)
        .await
        .expect("complete source row");
        sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, success_count, failure_count, terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete, total_tokens, total_cost) VALUES (?1, ?2, 2, 2, 0, 2, 200, 0.2, 1, 200, 0.2)",
        )
        .bind(hour_start)
        .bind(SOURCE_XY)
        .execute(&pool)
        .await
        .expect("contradictory canonical hourly rollup");
        sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 2, 200, 2, 0.2, 2)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("stale materialized daily rollup");
        sqlx::query(
            "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 2, 200, 2, 0.2, 2)",
        )
        .bind(hour_start)
        .execute(&pool)
        .await
        .expect("stale materialized hourly rollup");
        sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, statistics_start_date = ?2, last_integrity_audit_at = NULL WHERE id = ?3",
        )
        .bind(LONG_TERM_STATUS_READY)
        .bind(date.to_string())
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("make reconciliation due");

        refresh_long_term_stats(&pool, 400)
            .await
            .expect("contradictory source reconciliation is repaired");

        let (status, last_error) = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT status, last_error FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("long-term state");
        let proof = sqlx::query_scalar::<_, i64>(
            "SELECT terminal_proof_complete FROM invocation_rollup_hourly WHERE bucket_start_epoch = ?1 AND source = ?2",
        )
        .bind(hour_start)
        .bind(SOURCE_XY)
        .fetch_one(&pool)
        .await
        .expect("repaired terminal proof");
        let queue_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_stats_repair_queue WHERE stats_date = ?1",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("repair queue count");
        let repaired_daily_calls = sqlx::query_scalar::<_, i64>(
            "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("repaired daily rollup");

        assert_eq!(status, LONG_TERM_STATUS_READY);
        assert_eq!(last_error, None);
        assert_eq!(proof, 1);
        assert_eq!(queue_count, 0);
        assert_eq!(repaired_daily_calls, 1);
    }

    #[tokio::test]
    async fn refresh_hides_stats_when_terminal_proof_reconciliation_requires_a_missing_column() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("full schema");
        let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
        let (day_start, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
        let hour_start = day_start + 10 * 60 * 60;

        sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, success_count, failure_count, terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete, total_tokens, total_cost) VALUES (?1, 'trusted', 1, 1, 0, 1, 100, 0.1, 1, 100, 0.1)",
        )
        .bind(hour_start)
        .execute(&pool)
        .await
        .expect("trusted canonical hourly rollup");
        sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("materialized daily rollup");
        sqlx::query(
            "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(hour_start)
        .execute(&pool)
        .await
        .expect("materialized hourly rollup");
        sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, statistics_start_date = ?2, last_integrity_audit_at = NULL WHERE id = ?3",
        )
        .bind(LONG_TERM_STATUS_READY)
        .bind(date.to_string())
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("make reconciliation due");

        // The long-term query can read this legacy shape, but the canonical proof scan requires
        // `source`. Treating that failure as harmless would incorrectly publish `ready`.
        sqlx::query("DROP TABLE codex_invocations")
            .execute(&pool)
            .await
            .expect("replace invocation source table");
        sqlx::query(
            r#"
            CREATE TABLE codex_invocations (
                id INTEGER PRIMARY KEY,
                invoke_id TEXT,
                occurred_at TEXT NOT NULL,
                status TEXT,
                model TEXT,
                payload TEXT,
                total_tokens INTEGER,
                output_tokens INTEGER,
                cost REAL,
                t_total_ms REAL,
                t_req_read_ms REAL,
                t_req_parse_ms REAL,
                t_upstream_connect_ms REAL,
                t_upstream_ttfb_ms REAL,
                t_upstream_stream_ms REAL,
                error_message TEXT
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create long-term-readable legacy invocation schema");

        refresh_long_term_stats(&pool, 400)
            .await
            .expect("schema reconciliation failure is contained as an availability error");

        let (status, last_error) = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT status, last_error FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("long-term state after reconciliation failure");
        assert_eq!(status, LONG_TERM_STATUS_ERROR);
        assert_eq!(
            last_error.as_deref(),
            Some(LONG_TERM_TERMINAL_PROOF_UNAVAILABLE_ERROR)
        );
    }

    #[tokio::test]
    async fn refresh_retries_terminal_proof_reconciliation_after_source_availability_recovers() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("full schema");
        let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
        let (day_start, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
        let missing_archive_path = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-recoverable-trusted-source-{}-{}.sqlite.gz",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        let missing_archive_path = missing_archive_path.to_string_lossy().to_string();

        sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, success_count, failure_count, terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete, total_tokens, total_cost) VALUES (?1, 'trusted', 1, 1, 0, 1, 100, 0.1, 1, 100, 0.1)",
        )
        .bind(day_start + 10 * 60 * 60)
        .execute(&pool)
        .await
        .expect("trusted canonical hourly rollup");
        sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("materialized daily rollup");
        sqlx::query(
            "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(day_start + 10 * 60 * 60)
        .execute(&pool)
        .await
        .expect("materialized hourly rollup");
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                dataset,
                month_key,
                file_path,
                sha256,
                row_count,
                status,
                coverage_start_at,
                coverage_end_at,
                created_at
            )
            VALUES ('codex_invocations', '2026-01', ?1, 'recoverable-trusted-source-sha', 1, 'completed', ?2, ?2, datetime('now'))
            "#,
        )
        .bind(&missing_archive_path)
        .bind(format!("{date} 10:00:00"))
        .execute(&pool)
        .await
        .expect("missing completed archive manifest");
        sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, statistics_start_date = ?2, last_integrity_audit_at = NULL WHERE id = ?3",
        )
        .bind(LONG_TERM_STATUS_READY)
        .bind(date.to_string())
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("make first reconciliation due");

        refresh_long_term_stats(&pool, 400)
            .await
            .expect("missing source should become a retryable availability error");
        sqlx::query("DELETE FROM archive_batches WHERE file_path = ?1")
            .bind(&missing_archive_path)
            .execute(&pool)
            .await
            .expect("simulate restored source availability before the next hourly audit");

        refresh_long_term_stats(&pool, 400)
            .await
            .expect("next refresh should retry terminal proof reconciliation immediately");

        let (status, last_error) = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT status, last_error FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("long-term state after source recovery");
        assert_eq!(status, LONG_TERM_STATUS_READY);
        assert_eq!(last_error, None);
    }

    #[tokio::test]
    async fn refresh_keeps_error_after_proof_revocation_when_a_later_source_read_fails() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("full schema");
        let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
        let (day_start, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
        let missing_archive_path = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-missing-source-before-later-error-{}-{}.sqlite.gz",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        let missing_archive_path = missing_archive_path.to_string_lossy().to_string();

        sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, success_count, failure_count, terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete, total_tokens, total_cost) VALUES (?1, 'trusted', 1, 1, 0, 1, 100, 0.1, 1, 100, 0.1)",
        )
        .bind(day_start + 10 * 60 * 60)
        .execute(&pool)
        .await
        .expect("trusted canonical hourly rollup");
        sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("materialized daily rollup");
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                dataset,
                month_key,
                file_path,
                sha256,
                row_count,
                status,
                coverage_start_at,
                coverage_end_at,
                created_at
            )
            VALUES ('codex_invocations', '2026-01', ?1, 'missing-source-before-later-error', 1, 'completed', ?2, ?2, datetime('now'))
            "#,
        )
        .bind(&missing_archive_path)
        .bind(format!("{date} 10:00:00"))
        .execute(&pool)
        .await
        .expect("missing completed archive manifest");
        sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, statistics_start_date = ?2, last_integrity_audit_at = NULL WHERE id = ?3",
        )
        .bind(LONG_TERM_STATUS_READY)
        .bind(date.to_string())
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("make hourly audit due");

        // Reconciliation above revokes the proof; this malformed relation then fails a later
        // source read in the same refresh and exercises the fallback status path.
        sqlx::query("DROP TABLE pool_upstream_accounts")
            .execute(&pool)
            .await
            .expect("drop account source table");
        sqlx::query("CREATE TABLE pool_upstream_accounts (id INTEGER PRIMARY KEY)")
            .execute(&pool)
            .await
            .expect("create malformed account source table");

        refresh_long_term_stats(&pool, 400)
            .await
            .expect_err("later source read should fail after the proof is revoked");

        let status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("load long-term state after later error");
        assert_eq!(status, LONG_TERM_STATUS_ERROR);
    }

    #[tokio::test]
    async fn integrity_audit_detects_nonzero_rollups_without_canonical_hourly_rows() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        create_long_term_integrity_oracle(&pool).await;
        let date = NaiveDate::from_ymd_opt(2026, 7, 23).expect("fixed date");
        let (start_epoch, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
        sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("stale daily rollup");
        sqlx::query(
            "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(start_epoch + 10 * 60 * 60)
        .execute(&pool)
        .await
        .expect("stale hourly rollup");

        let mismatches = audit_long_term_integrity(&pool, date, date)
            .await
            .expect("audit stale rollups");
        assert_eq!(mismatches.len(), 1);
        assert_eq!(mismatches[0].date, date);
        assert_eq!(mismatches[0].expected.calls, 0);
        assert_eq!(mismatches[0].observed.calls, 1);
    }

    #[tokio::test]
    async fn integrity_audit_detects_dimension_rows_on_an_empty_canonical_day() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        create_long_term_integrity_oracle(&pool).await;
        let date = NaiveDate::from_ymd_opt(2026, 7, 23).expect("fixed date");
        let (start_epoch, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
        for (dimension, series_key, display_name) in [
            ("model", "model:v2:stale", "stale model"),
            ("upstream", "upstream:stale", "stale upstream"),
        ] {
            sqlx::query(
                "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, ?2, ?3, ?4, 1, 100, 1, 0.1, 1)",
            )
            .bind(date.to_string())
            .bind(dimension)
            .bind(series_key)
            .bind(display_name)
            .execute(&pool)
            .await
            .expect("stale dimension daily rollup");
            sqlx::query(
                "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, ?2, ?3, ?4, 1, 100, 1, 0.1, 1)",
            )
            .bind(start_epoch + 10 * 60 * 60)
            .bind(dimension)
            .bind(series_key)
            .bind(display_name)
            .execute(&pool)
            .await
            .expect("stale dimension hourly rollup");
        }

        let mismatches = audit_long_term_integrity(&pool, date, date)
            .await
            .expect("audit dimension-only stale rollups");
        assert_eq!(mismatches.len(), 1);
        assert_eq!(mismatches[0].date, date);
        assert_eq!(mismatches[0].expected, LongTermIntegrityTotals::default());
        assert_eq!(mismatches[0].observed, LongTermIntegrityTotals::default());
        assert!(mismatches[0].reason.contains("one or more dimensions"));
    }

    #[tokio::test]
    async fn integrity_audit_detects_dimension_rows_on_an_active_only_canonical_day() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        create_long_term_integrity_oracle(&pool).await;
        let date = NaiveDate::from_ymd_opt(2026, 7, 23).expect("fixed date");
        let (start_epoch, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
        sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete, total_tokens, total_cost) VALUES (?1, 'active-only', 1, 0, 0, 0, 1, 100, 0.1)",
        )
        .bind(start_epoch + 10 * 60 * 60)
        .execute(&pool)
        .await
        .expect("active-only canonical hourly rollup");
        for (dimension, series_key, display_name) in [
            ("model", "model:v2:stale", "stale model"),
            ("upstream", "upstream:stale", "stale upstream"),
        ] {
            sqlx::query(
                "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, ?2, ?3, ?4, 1, 100, 1, 0.1, 1)",
            )
            .bind(date.to_string())
            .bind(dimension)
            .bind(series_key)
            .bind(display_name)
            .execute(&pool)
            .await
            .expect("stale dimension daily rollup");
            sqlx::query(
                "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, ?2, ?3, ?4, 1, 100, 1, 0.1, 1)",
            )
            .bind(start_epoch + 10 * 60 * 60)
            .bind(dimension)
            .bind(series_key)
            .bind(display_name)
            .execute(&pool)
            .await
            .expect("stale dimension hourly rollup");
        }

        let mismatches = audit_long_term_integrity(&pool, date, date)
            .await
            .expect("audit active-only canonical day");
        assert_eq!(mismatches.len(), 1);
        assert_eq!(mismatches[0].date, date);
        assert_eq!(mismatches[0].expected, LongTermIntegrityTotals::default());
        assert_eq!(mismatches[0].observed, LongTermIntegrityTotals::default());
        assert!(mismatches[0].reason.contains("terminal totals are empty"));
    }

    #[tokio::test]
    async fn integrity_audit_keeps_zero_total_wall_time_continuations() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        create_long_term_integrity_oracle(&pool).await;
        let date = NaiveDate::from_ymd_opt(2026, 7, 23).expect("fixed date");
        let (start_epoch, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");

        // A call that began before midnight may contribute wall time to this date without
        // contributing a canonical terminal invocation bucket of its own.
        sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, wall_time_ms, wall_time_samples) VALUES (?1, 'overall', 'overall', '全部调用', 60000, 1)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("wall-time continuation daily rollup");
        sqlx::query(
            "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, wall_time_ms, wall_time_samples) VALUES (?1, 'overall', 'overall', '全部调用', 60000, 1)",
        )
        .bind(start_epoch)
        .execute(&pool)
        .await
        .expect("wall-time continuation hourly rollup");

        let mismatches = audit_long_term_integrity(&pool, date, date)
            .await
            .expect("audit zero-total wall-time continuation");
        assert!(
            mismatches.is_empty(),
            "empty canonical totals must not erase a valid cross-day wall-time continuation"
        );
    }

    #[tokio::test]
    async fn refresh_skips_audit_before_the_persisted_reconstructable_start() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        create_long_term_test_invocations(&pool).await;
        create_long_term_integrity_oracle(&pool).await;
        let today = Utc::now().with_timezone(&Shanghai).date_naive();
        let unavailable_date = today - ChronoDuration::days(5);
        let reconstructable_start = today - ChronoDuration::days(3);
        let (hour_start, _) =
            long_term_day_epoch_bounds(unavailable_date).expect("Shanghai day bounds");
        sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete, total_tokens, total_cost) VALUES (?1, 'canonical', 2, 2, 200, 0.2, 0, 200, 0.2)",
        )
        .bind(hour_start + 10 * 60 * 60)
        .execute(&pool)
        .await
        .expect("canonical unavailable-prefix hour");
        sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(unavailable_date.to_string())
        .execute(&pool)
        .await
        .expect("stale unavailable-prefix daily rollup");
        sqlx::query(
            "INSERT INTO long_term_stats_repair_queue (stats_date, expected_calls, expected_token_total, expected_cost_total, observed_calls, observed_token_total, observed_cost_total, last_error) VALUES (?1, 2, 200, 0.2, 1, 100, 0.1, 'unreadable archive prefix')",
        )
        .bind(unavailable_date.to_string())
        .execute(&pool)
        .await
        .expect("inactive unavailable-prefix repair");
        sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, statistics_start_date = ?2, last_integrity_audit_at = NULL WHERE id = ?3",
        )
        .bind(LONG_TERM_STATUS_READY)
        .bind(reconstructable_start.to_string())
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("persist reconstructable suffix");

        refresh_long_term_stats(&pool, 400)
            .await
            .expect("refresh valid suffix");

        let queued_repairs =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_stats_repair_queue")
                .fetch_one(&pool)
                .await
                .expect("queued repair count");
        let attempts = sqlx::query_scalar::<_, i64>(
            "SELECT attempts FROM long_term_stats_repair_queue WHERE stats_date = ?1",
        )
        .bind(unavailable_date.to_string())
        .fetch_one(&pool)
        .await
        .expect("inactive repair attempts");
        let status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("long-term status");
        assert_eq!(queued_repairs, 1);
        assert_eq!(attempts, 0);
        assert_eq!(status, LONG_TERM_STATUS_READY);
    }

    #[test]
    fn targeted_repair_merges_archive_details_into_retained_live_row() {
        let retained_live = LongTermInvocationRow {
            id: 1,
            invoke_id: Some("same-invocation".to_string()),
            occurred_at: "2026-07-26 10:00:00".to_string(),
            status: Some("success".to_string()),
            model: None,
            request_model: None,
            response_model: None,
            reasoning_effort: None,
            upstream_account_id: None,
            upstream_account_kind: None,
            upstream_account_name: None,
            total_tokens: Some(100),
            output_tokens: Some(50),
            cost: Some(1.5),
            t_total_ms: Some(900.0),
            t_req_read_ms: Some(100.0),
            t_req_parse_ms: Some(50.0),
            t_upstream_connect_ms: Some(100.0),
            t_upstream_ttfb_ms: Some(300.0),
            t_upstream_stream_ms: Some(600.0),
            error_message: None,
        };
        let mut archived = LongTermInvocationRow {
            model: Some("archive-model".to_string()),
            request_model: Some("request-model".to_string()),
            response_model: Some("response-model".to_string()),
            reasoning_effort: Some("high".to_string()),
            upstream_account_id: Some(42),
            ..retained_live.clone()
        };

        merge_long_term_invocation_row(&mut archived, &retained_live);

        assert_eq!(archived.status.as_deref(), Some("success"));
        assert_eq!(archived.model.as_deref(), Some("archive-model"));
        assert_eq!(archived.response_model.as_deref(), Some("response-model"));
        assert_eq!(archived.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(archived.upstream_account_id, Some(42));
    }

    #[tokio::test]
    async fn incremental_projection_prunes_expired_hourly_rollups_and_intervals() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        let retention_start = long_term_projection_hourly_retention_start_date(366);
        let expired_date = retention_start.pred_opt().expect("expired date");
        let retained_date = retention_start;
        let epoch = |date: NaiveDate| {
            date.and_hms_opt(0, 0, 0)
                .and_then(|value| Shanghai.from_local_datetime(&value).single())
                .expect("shanghai hour")
                .timestamp()
        };
        let expired_epoch = epoch(expired_date);
        let retained_epoch = epoch(retained_date);
        for (bucket_start_epoch, key) in [(expired_epoch, "expired"), (retained_epoch, "retained")]
        {
            let bucket_date = Shanghai
                .timestamp_opt(bucket_start_epoch, 0)
                .single()
                .expect("shanghai bucket")
                .date_naive();
            sqlx::query(
                "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name) VALUES (?1, 'overall', ?2, 'All')",
            )
            .bind(bucket_start_epoch)
            .bind(key)
            .execute(&pool)
            .await
            .expect("hourly rollup");
            sqlx::query(
                "INSERT INTO long_term_projection_intervals (bucket_kind, bucket_date, bucket_key, dimension, series_key, invocation_row_id, interval_start_ms, interval_end_ms) VALUES ('hourly', ?1, ?2, 'overall', ?3, 1, 0, 1)",
            )
            .bind(bucket_date.to_string())
            .bind(bucket_start_epoch.to_string())
            .bind(key)
            .execute(&pool)
            .await
            .expect("hourly interval");
        }
        sqlx::query(
            "INSERT INTO long_term_projection_intervals (bucket_kind, bucket_date, bucket_key, dimension, series_key, invocation_row_id, interval_start_ms, interval_end_ms) VALUES ('daily', ?1, ?1, 'overall', 'daily-retained', 1, 0, 1)",
        )
        .bind(expired_date.to_string())
        .execute(&pool)
        .await
        .expect("daily interval");

        let (pruned_hourly_rows, pruned_interval_rows) =
            prune_long_term_projection_hourly_retention(&pool, 366)
                .await
                .expect("prune hourly retention");

        assert_eq!(pruned_hourly_rows, 1);
        assert_eq!(pruned_interval_rows, 1);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_usage_hourly")
                .fetch_one(&pool)
                .await
                .expect("retained hourly count"),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_intervals")
                .fetch_one(&pool)
                .await
                .expect("retained interval count"),
            2
        );
    }

    #[test]
    fn metrics_keep_call_count_separate_from_success_only_timing_samples() {
        let success = LongTermInvocationRow {
            id: 1,
            invoke_id: None,
            occurred_at: "2026-07-26T00:00:00Z".to_string(),
            status: Some("success".to_string()),
            model: Some("legacy-model".to_string()),
            request_model: Some("request-model".to_string()),
            response_model: Some("response-model".to_string()),
            reasoning_effort: Some("high".to_string()),
            upstream_account_id: None,
            upstream_account_kind: None,
            upstream_account_name: None,
            total_tokens: Some(100),
            output_tokens: Some(50),
            cost: Some(1.5),
            t_total_ms: Some(900.0),
            t_req_read_ms: Some(100.0),
            t_req_parse_ms: Some(50.0),
            t_upstream_connect_ms: Some(100.0),
            t_upstream_ttfb_ms: Some(300.0),
            t_upstream_stream_ms: Some(600.0),
            error_message: None,
        };
        let failure = LongTermInvocationRow {
            id: 2,
            invoke_id: None,
            occurred_at: "2026-07-26T00:00:01Z".to_string(),
            status: Some("failed".to_string()),
            model: None,
            request_model: None,
            response_model: None,
            reasoning_effort: None,
            upstream_account_id: None,
            upstream_account_kind: None,
            upstream_account_name: None,
            total_tokens: None,
            output_tokens: Some(10),
            cost: None,
            t_total_ms: Some(100.0),
            t_req_read_ms: None,
            t_req_parse_ms: None,
            t_upstream_connect_ms: None,
            t_upstream_ttfb_ms: Some(20.0),
            t_upstream_stream_ms: Some(80.0),
            error_message: None,
        };
        let mut accumulator = LongTermAccumulator::default();
        accumulator.add_call(&success, Some((0, 900)));
        accumulator.add_call(&failure, None);
        let metrics = LongTermMetrics::from_accumulator(&accumulator);
        assert_eq!(metrics.calls, 2);
        assert_eq!(metrics.tokens, Some(100));
        assert_eq!(metrics.cost, Some(1.5));
        assert_eq!(metrics.usage_time_ms, Some(900.0));
        assert_eq!(metrics.output_speed_tokens_per_second, Some(50.0 / 0.6));
        assert_eq!(metrics.first_byte_ms, Some(550.0));
        assert_eq!(metrics.response_ms, Some(600.0));
        assert_eq!(normalize_long_term_model(&success), "response-model");
        assert_eq!(normalize_long_term_model(&failure), "未知模型");
        let mut blank_legacy = success.clone();
        blank_legacy.response_model = None;
        blank_legacy.model = Some("  \t".to_string());
        assert_eq!(normalize_long_term_model(&blank_legacy), "request-model");
    }

    #[test]
    fn non_api_key_upstreams_share_the_other_series() {
        let row = LongTermInvocationRow {
            id: 1,
            invoke_id: None,
            occurred_at: "2026-07-26T00:00:00Z".to_string(),
            status: Some("success".to_string()),
            model: None,
            request_model: Some("gpt-5".to_string()),
            response_model: None,
            reasoning_effort: None,
            upstream_account_id: Some(7),
            upstream_account_kind: Some("oauth_codex".to_string()),
            upstream_account_name: Some("OAuth".to_string()),
            total_tokens: Some(1),
            output_tokens: Some(1),
            cost: Some(0.1),
            t_total_ms: None,
            t_req_read_ms: None,
            t_req_parse_ms: None,
            t_upstream_connect_ms: None,
            t_upstream_ttfb_ms: None,
            t_upstream_stream_ms: None,
            error_message: None,
        };
        assert_eq!(
            normalize_long_term_upstream(&row),
            ("other".to_string(), "其他".to_string())
        );
    }

    #[tokio::test]
    async fn refresh_replaces_stale_rows_when_the_complete_live_source_is_empty() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        create_long_term_test_invocations(&pool).await;
        let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(1);
        let (day_start, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
        create_long_term_integrity_oracle(&pool).await;
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, model, payload, total_tokens, output_tokens, cost, t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms) VALUES (1, 'test-invoke-1', ?1, 'success', 'gpt-5', '{\"reasoningEffort\":\"high\"}', 12, 4, 0.2, 100, 10, 5, 5, 20, 80)",
        )
        .bind(format!("{date}T12:00:00+08:00"))
        .execute(&pool)
        .await
        .expect("invocation row");
        sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, terminal_count, terminal_tokens, terminal_cost, total_tokens, total_cost) VALUES (?1, 'canonical', 1, 1, 12, 0.2, 12, 0.2)",
        )
        .bind(day_start + 12 * 60 * 60)
        .execute(&pool)
        .await
        .expect("canonical hourly proof");
        refresh_long_term_stats(&pool, 400).await.expect("refresh");
        let daily_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_usage_daily WHERE dimension = 'overall'",
        )
        .fetch_one(&pool)
        .await
        .expect("daily count");
        assert_eq!(daily_rows, 1);
        let status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM long_term_stats_state WHERE id = 1",
        )
        .fetch_one(&pool)
        .await
        .expect("state after initial refresh");
        assert_eq!(status, LONG_TERM_STATUS_READY);
        sqlx::query("DELETE FROM codex_invocations")
            .execute(&pool)
            .await
            .expect("remove live source row");
        refresh_long_term_stats(&pool, 400)
            .await
            .expect("refresh after source removal");
        let remaining_daily_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_usage_daily WHERE dimension = 'overall'",
        )
        .fetch_one(&pool)
        .await
        .expect("remaining daily count");
        assert_eq!(remaining_daily_rows, 0);
        let remaining_hourly_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_usage_hourly WHERE dimension = 'overall'",
        )
        .fetch_one(&pool)
        .await
        .expect("remaining hourly count");
        assert_eq!(remaining_hourly_rows, 0);
        let status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM long_term_stats_state WHERE id = 1",
        )
        .fetch_one(&pool)
        .await
        .expect("state");
        assert_eq!(status, LONG_TERM_STATUS_READY);
    }

    #[tokio::test]
    async fn refresh_does_not_overwrite_complete_rollups_with_partial_rebuilds() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        sqlx::query(
            r#"
            CREATE TABLE codex_invocations (
                id INTEGER PRIMARY KEY,
                invoke_id TEXT,
                occurred_at TEXT NOT NULL,
                status TEXT,
                model TEXT,
                payload TEXT,
                total_tokens INTEGER,
                output_tokens INTEGER,
                cost REAL,
                t_total_ms REAL,
                t_req_read_ms REAL,
                t_req_parse_ms REAL,
                t_upstream_connect_ms REAL,
                t_upstream_ttfb_ms REAL,
                t_upstream_stream_ms REAL,
                error_message TEXT
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("invocation schema");
        let today = Utc::now().with_timezone(&Shanghai).date_naive();
        let occurred_at = format!("{today}T12:00:00+08:00");
        let day_start_epoch = today
            .and_hms_opt(0, 0, 0)
            .and_then(|value| Shanghai.from_local_datetime(&value).single())
            .expect("Shanghai start")
            .timestamp();
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, model, payload, total_tokens, output_tokens, cost) VALUES (1, 'partial', ?1, 'success', 'gpt-5', '{}', 7, 2, 0.07)",
        )
        .bind(occurred_at)
        .execute(&pool)
        .await
        .expect("partial source row");
        sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 10, 100, 10, 1.0, 10)",
        )
        .bind(today.to_string())
        .execute(&pool)
        .await
        .expect("complete daily rollup");
        sqlx::query(
            "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 10, 100, 10, 1.0, 10)",
        )
        .bind(day_start_epoch + 12 * 60 * 60)
        .execute(&pool)
        .await
        .expect("complete hourly rollup");
        sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
            .bind(LONG_TERM_STATUS_READY)
            .bind(LONG_TERM_STATE_ID)
            .execute(&pool)
            .await
            .expect("ready state");

        refresh_long_term_stats(&pool, 400)
            .await
            .expect("refresh protects complete rollups");

        let daily_calls = sqlx::query_scalar::<_, i64>(
            "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(today.to_string())
        .fetch_one(&pool)
        .await
        .expect("retained daily calls");
        let hourly_calls = sqlx::query_scalar::<_, i64>(
            "SELECT calls FROM long_term_usage_hourly WHERE bucket_start_epoch = ?1 AND dimension = 'overall'",
        )
        .bind(day_start_epoch + 12 * 60 * 60)
        .fetch_one(&pool)
        .await
        .expect("retained hourly calls");
        assert_eq!(daily_calls, 10);
        assert_eq!(hourly_calls, 10);
    }

    #[tokio::test]
    async fn refresh_rebuilds_completed_rollups_after_complete_source_reconciliation() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        create_long_term_test_invocations(&pool).await;
        create_long_term_integrity_oracle(&pool).await;
        let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(1);
        let (day_start, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
        let hour_start = day_start + 10 * 60 * 60;
        insert_long_term_test_invocation(&pool, 1, format!("{date}T10:00:00+08:00")).await;
        sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete, total_tokens, total_cost) VALUES (?1, 'legacy', 2, 2, 200, 0.2, 0, 200, 0.2)",
        )
        .bind(hour_start)
        .execute(&pool)
        .await
        .expect("untrusted canonical rollup");
        sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 2, 200, 2, 0.2, 2)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("durable daily rollup");
        sqlx::query(
            "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 2, 200, 2, 0.2, 2)",
        )
        .bind(hour_start)
        .execute(&pool)
        .await
        .expect("durable hourly rollup");
        sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
            .bind(LONG_TERM_STATUS_READY)
            .bind(LONG_TERM_STATE_ID)
            .execute(&pool)
            .await
            .expect("ready state");

        refresh_long_term_stats(&pool, 400)
            .await
            .expect("refresh after complete source reconciliation");

        let daily = sqlx::query_as::<_, (i64, i64, f64)>(
            "SELECT calls, token_total, cost_total FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("rebuilt daily rollup");
        let hourly = sqlx::query_as::<_, (i64, i64, f64)>(
            "SELECT calls, token_total, cost_total FROM long_term_usage_hourly WHERE bucket_start_epoch = ?1 AND dimension = 'overall'",
        )
        .bind(hour_start)
        .fetch_one(&pool)
        .await
        .expect("rebuilt hourly rollup");
        let queued_repairs =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_stats_repair_queue")
                .fetch_one(&pool)
                .await
                .expect("repair queue count");
        assert_eq!(daily, (1, 100, 0.1));
        assert_eq!(hourly, (1, 100, 0.1));
        assert_eq!(queued_repairs, 0);
    }

    #[tokio::test]
    async fn full_rebuild_hides_completed_days_without_trusted_integrity_proof() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        create_long_term_test_invocations(&pool).await;
        let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
        insert_long_term_test_invocation(&pool, 1, format!("{date}T10:00:00+08:00")).await;

        refresh_long_term_stats(&pool, 400)
            .await
            .expect("defer full rebuild candidate without canonical proof");

        let materialized_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_usage_daily WHERE stats_date = ?1",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("count unpublished daily rollups");
        let queued_repairs = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_stats_repair_queue WHERE stats_date = ?1",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("count deferred repair");
        let state = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT status, statistics_start_date FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("load failed full rebuild state");

        assert_eq!(materialized_rows, 0);
        assert_eq!(queued_repairs, 1);
        assert_eq!(state.0, LONG_TERM_STATUS_ERROR);
        let expected_start = date.to_string();
        assert_eq!(state.1.as_deref(), Some(expected_start.as_str()));
    }

    #[tokio::test]
    async fn refresh_audits_and_repairs_historical_long_term_rollups_idempotently() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        create_long_term_test_invocations(&pool).await;
        create_long_term_integrity_oracle(&pool).await;
        let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
        let (first_hour, second_hour) = seed_long_term_integrity_case(&pool, date, 2).await;

        refresh_long_term_stats(&pool, 400)
            .await
            .expect("historical integrity repair");

        let daily = sqlx::query_as::<_, (i64, i64, f64)>(
            "SELECT calls, token_total, cost_total FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("repaired daily rollup");
        assert_eq!(daily.0, 2);
        assert_eq!(daily.1, 200);
        assert!((daily.2 - 0.2).abs() < 1e-9);
        let hourly = sqlx::query_as::<_, (i64, i64)>(
            "SELECT bucket_start_epoch, calls FROM long_term_usage_hourly WHERE dimension = 'overall' AND bucket_start_epoch IN (?1, ?2) ORDER BY bucket_start_epoch",
        )
        .bind(first_hour)
        .bind(second_hour)
        .fetch_all(&pool)
        .await
        .expect("repaired hourly rollups");
        assert_eq!(hourly, vec![(first_hour, 1), (second_hour, 1)]);
        let model_calls = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(SUM(calls), 0) FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'model'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("model dimension");
        let upstream_calls = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(SUM(calls), 0) FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'upstream'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("upstream dimension");
        assert_eq!(model_calls, 2);
        assert_eq!(upstream_calls, 2);
        let queue_count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_stats_repair_queue")
                .fetch_one(&pool)
                .await
                .expect("repair queue count");
        let status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("repair status");
        assert_eq!(queue_count, 0);
        assert_eq!(status, LONG_TERM_STATUS_READY);

        refresh_long_term_stats(&pool, 400)
            .await
            .expect("repeat historical integrity repair");
        let repeated_daily = sqlx::query_as::<_, (i64, i64, f64)>(
            "SELECT calls, token_total, cost_total FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("idempotent daily rollup");
        let repeated_hourly = sqlx::query_as::<_, (i64, i64)>(
            "SELECT bucket_start_epoch, calls FROM long_term_usage_hourly WHERE dimension = 'overall' AND bucket_start_epoch IN (?1, ?2) ORDER BY bucket_start_epoch",
        )
        .bind(first_hour)
        .bind(second_hour)
        .fetch_all(&pool)
        .await
        .expect("idempotent hourly rollups");
        assert_eq!(repeated_daily, daily);
        assert_eq!(repeated_hourly, hourly);
    }

    #[tokio::test]
    async fn refresh_repairs_bad_rollups_after_complete_source_reconciliation() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        create_long_term_test_invocations(&pool).await;
        create_long_term_integrity_oracle(&pool).await;
        let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
        seed_long_term_integrity_case(&pool, date, 1).await;

        refresh_long_term_stats(&pool, 400)
            .await
            .expect("complete source repair");

        let repaired_calls = sqlx::query_scalar::<_, i64>(
            "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("repaired daily rollup");
        let queue_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_stats_repair_queue WHERE stats_date = ?1",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("repair queue count");
        let status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("ready state");
        assert_eq!(repaired_calls, 1);
        assert_eq!(queue_count, 0);
        assert_eq!(status, LONG_TERM_STATUS_READY);
    }

    #[tokio::test]
    async fn refresh_replaces_a_stale_day_with_empty_canonical_totals() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        create_long_term_test_invocations(&pool).await;
        create_long_term_integrity_oracle(&pool).await;
        let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
        let (day_start, day_end) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
        sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("stale daily rollup");
        sqlx::query(
            "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(day_start + 10 * 60 * 60)
        .execute(&pool)
        .await
        .expect("stale hourly rollup");
        for (dimension, series_key, display_name) in [
            ("model", "model:v2:stale", "stale model"),
            ("upstream", "upstream:stale", "stale upstream"),
        ] {
            sqlx::query(
                "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, ?2, ?3, ?4, 1, 100, 1, 0.1, 1)",
            )
            .bind(date.to_string())
            .bind(dimension)
            .bind(series_key)
            .bind(display_name)
            .execute(&pool)
            .await
            .expect("stale dimension daily rollup");
            sqlx::query(
                "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, ?2, ?3, ?4, 1, 100, 1, 0.1, 1)",
            )
            .bind(day_start + 10 * 60 * 60)
            .bind(dimension)
            .bind(series_key)
            .bind(display_name)
            .execute(&pool)
            .await
            .expect("stale dimension hourly rollup");
        }
        sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
            .bind(LONG_TERM_STATUS_READY)
            .bind(LONG_TERM_STATE_ID)
            .execute(&pool)
            .await
            .expect("ready state");

        refresh_long_term_stats(&pool, 400)
            .await
            .expect("repair empty canonical day");

        let daily_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_usage_daily WHERE stats_date = ?1",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("empty repaired daily rows");
        let hourly_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_usage_hourly WHERE bucket_start_epoch >= ?1 AND bucket_start_epoch < ?2",
        )
        .bind(day_start)
        .bind(day_end)
        .fetch_one(&pool)
        .await
        .expect("empty repaired hourly rows");
        let queue_count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_stats_repair_queue")
                .fetch_one(&pool)
                .await
                .expect("cleared repair queue");
        let status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("recovered status");
        assert_eq!(daily_rows, 0);
        assert_eq!(hourly_rows, 0);
        assert_eq!(queue_count, 0);
        assert_eq!(status, LONG_TERM_STATUS_READY);
    }

    #[tokio::test]
    async fn refresh_applies_backoff_when_a_queued_repair_is_source_incomplete() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        create_long_term_test_invocations(&pool).await;
        let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
        insert_long_term_test_invocation(&pool, 1, format!("{date}T10:00:00+08:00")).await;
        sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 2, 200, 2, 0.2, 2)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("durable complete daily rollup");
        sqlx::query(
            "INSERT INTO long_term_stats_repair_queue (stats_date, expected_calls, expected_token_total, expected_cost_total, observed_calls, observed_token_total, observed_cost_total, last_error) VALUES (?1, 2, 200, 0.2, 1, 100, 0.1, 'source data unavailable')",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("due repair queue entry");
        sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
            .bind(LONG_TERM_STATUS_READY)
            .bind(LONG_TERM_STATE_ID)
            .execute(&pool)
            .await
            .expect("ready state");

        refresh_long_term_stats(&pool, 400)
            .await
            .expect("incomplete repair is deferred");

        let first = sqlx::query_as::<_, (i64, i64, String)>(
            "SELECT attempts, expected_calls, next_retry_at FROM long_term_stats_repair_queue WHERE stats_date = ?1",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("backed-off repair queue entry");
        let durable_calls = sqlx::query_scalar::<_, i64>(
            "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("preserved daily rollup");
        assert_eq!(first.0, 1);
        assert_eq!(first.1, 2);
        assert!(!first.2.is_empty());
        assert_eq!(durable_calls, 2);

        refresh_long_term_stats(&pool, 400)
            .await
            .expect("backoff suppresses immediate retry");
        let attempts = sqlx::query_scalar::<_, i64>(
            "SELECT attempts FROM long_term_stats_repair_queue WHERE stats_date = ?1",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("persistent retry attempts");
        assert_eq!(attempts, 1);
    }

    #[tokio::test]
    async fn refresh_replaces_a_queued_repair_with_an_empty_complete_source() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        create_long_term_test_invocations(&pool).await;
        create_long_term_integrity_oracle(&pool).await;
        let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
        let (day_start_epoch, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
        sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, terminal_count, terminal_tokens, terminal_cost, total_tokens, total_cost) VALUES (?1, 'canonical', 1, 1, 100, 0.1, 100, 0.1)",
        )
        .bind(day_start_epoch + 10 * 60 * 60)
        .execute(&pool)
        .await
        .expect("canonical hourly rollup");
        sqlx::query(
            "INSERT INTO long_term_stats_repair_queue (stats_date, expected_calls, expected_token_total, expected_cost_total, observed_calls, observed_token_total, observed_cost_total, last_error) VALUES (?1, 1, 100, 0.1, 0, 0, 0, 'source data unavailable')",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("queued repair");
        sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
            .bind(LONG_TERM_STATUS_ERROR)
            .bind(LONG_TERM_STATE_ID)
            .execute(&pool)
            .await
            .expect("error state");

        refresh_long_term_stats(&pool, 400)
            .await
            .expect("empty complete source is reconciled");

        let first_status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("pending error state");
        let queue_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_stats_repair_queue WHERE stats_date = ?1",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("queued repair count");
        let empty_daily_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("empty daily row count");
        assert_eq!(first_status, LONG_TERM_STATUS_EMPTY);
        assert_eq!(queue_count, 0);
        assert_eq!(empty_daily_rows, 0);

        insert_long_term_test_invocation(&pool, 1, format!("{date}T10:00:00+08:00")).await;

        refresh_long_term_stats(&pool, 400)
            .await
            .expect("refresh materializes after source arrives");

        let repaired_calls = sqlx::query_scalar::<_, i64>(
            "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("repaired daily rollup");
        let queue_count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_stats_repair_queue")
                .fetch_one(&pool)
                .await
                .expect("cleared repair queue");
        let final_status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("recovered ready state");
        assert_eq!(repaired_calls, 1);
        assert_eq!(queue_count, 0);
        assert_eq!(final_status, LONG_TERM_STATUS_READY);
    }

    #[tokio::test]
    async fn refresh_recovers_after_a_real_sqlite_lock_releases() {
        let (pool, db_url, db_path) = long_term_file_backed_pool("long-term-lock-release").await;
        create_long_term_test_invocations(&pool).await;
        let today = Utc::now().with_timezone(&Shanghai).date_naive();
        insert_long_term_test_invocation(&pool, 1, format!("{today}T12:00:00+08:00")).await;

        let mut lock_connection = SqliteConnection::connect(&db_url)
            .await
            .expect("connect lock holder");
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut lock_connection)
            .await
            .expect("acquire sqlite write lock");
        let refresh_pool = pool.clone();
        let refresh_task =
            tokio::spawn(async move { refresh_long_term_stats(&refresh_pool, 400).await });

        sleep(Duration::from_millis(125)).await;
        sqlx::query("COMMIT")
            .execute(&mut lock_connection)
            .await
            .expect("release sqlite write lock");
        refresh_task
            .await
            .expect("join refresh task")
            .expect("refresh should retry after lock release");

        let calls = sqlx::query_scalar::<_, i64>(
            "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(today.to_string())
        .fetch_one(&pool)
        .await
        .expect("published daily rollup");
        assert_eq!(calls, 1);

        lock_connection.close().await.expect("close lock holder");
        cleanup_long_term_file_backed_pool(pool, db_path).await;
    }

    #[tokio::test]
    async fn refresh_exhausts_real_sqlite_locks_without_partial_publication() {
        let (pool, db_url, db_path) = long_term_file_backed_pool("long-term-lock-exhaustion").await;
        create_long_term_test_invocations(&pool).await;
        let today = Utc::now().with_timezone(&Shanghai).date_naive();
        insert_long_term_test_invocation(&pool, 1, format!("{today}T12:00:00+08:00")).await;
        sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 10, 1000, 10, 1.0, 10)",
        )
        .bind(today.to_string())
        .execute(&pool)
        .await
        .expect("durable daily rollup");

        let mut lock_connection = SqliteConnection::connect(&db_url)
            .await
            .expect("connect lock holder");
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut lock_connection)
            .await
            .expect("acquire sqlite write lock");
        let error = refresh_long_term_stats(&pool, 400)
            .await
            .expect_err("persistent sqlite lock should exhaust retry budget");
        assert!(crate::is_sqlite_lock_error(&error));

        sqlx::query("ROLLBACK")
            .execute(&mut lock_connection)
            .await
            .expect("release sqlite write lock");
        let calls = sqlx::query_scalar::<_, i64>(
            "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(today.to_string())
        .fetch_one(&pool)
        .await
        .expect("preserved daily rollup");
        let hourly_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_usage_hourly WHERE bucket_start_epoch >= ?1 AND bucket_start_epoch < ?2",
        )
        .bind(
            long_term_day_epoch_bounds(today)
                .expect("Shanghai day bounds")
                .0,
        )
        .bind(
            long_term_day_epoch_bounds(today)
                .expect("Shanghai day bounds")
                .1,
        )
        .fetch_one(&pool)
        .await
        .expect("no partial hourly rollup");
        assert_eq!(calls, 10);
        assert_eq!(hourly_rows, 0);

        lock_connection.close().await.expect("close lock holder");
        cleanup_long_term_file_backed_pool(pool, db_path).await;
    }

    #[tokio::test]
    async fn long_term_refresh_retries_a_sqlite_lock_until_the_write_is_available() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let operation_attempts = Arc::clone(&attempts);
        let outcome = run_long_term_refresh_with_retry_delays(
            move || {
                let operation_attempts = Arc::clone(&operation_attempts);
                async move {
                    let attempt = operation_attempts.fetch_add(1, Ordering::SeqCst) + 1;
                    if attempt < 3 {
                        Err(anyhow::anyhow!("database is locked"))
                    } else {
                        Ok(attempt)
                    }
                }
            },
            &[Duration::ZERO, Duration::ZERO, Duration::ZERO],
        )
        .await
        .expect("lock should clear before retry budget is exhausted");
        assert_eq!(outcome, 3);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn long_term_refresh_stops_after_its_bounded_sqlite_lock_retry_budget() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let operation_attempts = Arc::clone(&attempts);
        let error = run_long_term_refresh_with_retry_delays(
            move || {
                let operation_attempts = Arc::clone(&operation_attempts);
                async move {
                    operation_attempts.fetch_add(1, Ordering::SeqCst);
                    Err::<(), _>(anyhow::anyhow!("database is locked"))
                }
            },
            &[Duration::ZERO, Duration::ZERO, Duration::ZERO],
        )
        .await
        .expect_err("persistent locks must exhaust the bounded retry budget");
        assert!(crate::is_sqlite_lock_error(&error));
        assert_eq!(attempts.load(Ordering::SeqCst), 4);
    }
}
