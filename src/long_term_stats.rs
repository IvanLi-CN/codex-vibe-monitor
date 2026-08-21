use super::*;
use sha2::{Digest, Sha256};

const LONG_TERM_TIMEZONE: &str = "Asia/Shanghai";
const LONG_TERM_STATE_ID: i64 = 1;
const LONG_TERM_STATUS_DISABLED: &str = "disabled";
const LONG_TERM_STATUS_PREPARING: &str = "preparing";
const LONG_TERM_STATUS_RUNNING: &str = "running";
const LONG_TERM_STATUS_READY: &str = "ready";
const LONG_TERM_STATUS_EMPTY: &str = "empty";
const LONG_TERM_STATUS_ERROR: &str = "error";
const LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR: &str =
    "long-term initial materialization is incomplete";
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
const LONG_TERM_PROJECTION_LOCK_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_millis(250),
    Duration::from_secs(1),
    Duration::from_secs(3),
];
pub(crate) const LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET: &str = "long_term_usage_stats";

static LONG_TERM_REFRESH_LOCK: once_cell::sync::Lazy<Mutex<()>> =
    once_cell::sync::Lazy::new(|| Mutex::new(()));
static LONG_TERM_PROJECTION_PUBLICATION_SEQUENCE: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

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
    next_repair_at: Option<Instant>,
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
struct LongTermProjectionLegacyIntervalRow {
    invocation_row_id: i64,
    bucket_kind: String,
    bucket_date: String,
    bucket_key: String,
    dimension: String,
    series_key: String,
    interval_start_ms: i64,
    interval_end_ms: i64,
}

#[derive(Debug, Clone)]
struct LongTermProjectionIntervalSegment {
    invocation_row_id: i64,
    model_series_key: String,
    upstream_series_key: String,
    interval_start_ms: i64,
    interval_end_ms: i64,
}

#[derive(Debug, Clone, FromRow)]
struct LongTermProjectionIntervalStateRow {
    invocation_row_id: i64,
    model_series_key: String,
    upstream_series_key: String,
    interval_start_ms: i64,
    interval_end_ms: i64,
}

#[derive(Debug, Clone, FromRow)]
struct LongTermProjectionLegacyCompactRow {
    invocation_row_id: i64,
    model_series_key: Option<String>,
    upstream_series_key: Option<String>,
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
        ensure_long_term_archive_source_identity(
            pool,
            "pool_upstream_request_attempts",
            &archive_path.file_path,
            &archive_path.sha256,
        )
        .await
        .with_context(|| {
            format!(
                "{LONG_TERM_ATTEMPT_ARCHIVE_UNAVAILABLE_ERROR}: {}",
                archive_path.file_path
            )
        })?;
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
        ensure_long_term_archive_source_identity(
            pool,
            "pool_upstream_request_attempts",
            &archive_path.file_path,
            &archive_path.sha256,
        )
        .await
        .with_context(|| {
            format!(
                "{LONG_TERM_ATTEMPT_ARCHIVE_UNAVAILABLE_ERROR}: {}",
                archive_path.file_path
            )
        })?;
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
    let query = long_term_archive_invocation_query_parts(pool).await?;
    Ok(format!(
        "{} WHERE {} ORDER BY occurred_at ASC, id ASC",
        query.select, query.terminal_filter
    ))
}

#[derive(Debug)]
struct LongTermArchiveInvocationRangeQueries {
    canonical: String,
    crossing_text: String,
    rfc3339: String,
    parts: LongTermArchiveInvocationQueryParts,
}

#[derive(Debug)]
struct LongTermArchiveInvocationQueryParts {
    select: String,
    terminal_filter: String,
    status_column: String,
    t_total_ms_column: String,
}

#[derive(Debug, Clone, PartialEq)]
struct LongTermArchiveCompatibility {
    has_legacy_crossing: bool,
    legacy_max_duration_ms: Option<f64>,
    legacy_min_occurred_at: Option<String>,
    has_rfc3339: bool,
    rfc3339_max_duration_ms: Option<f64>,
    rfc3339_min_occurred_at: Option<String>,
}

#[derive(Debug, Clone)]
struct LongTermRfc3339Compatibility {
    max_duration_ms: Option<f64>,
}

#[derive(Debug, FromRow)]
struct LongTermArchiveCompatibilityRow {
    id: i64,
    occurred_at: String,
    status: Option<String>,
    t_total_ms: Option<f64>,
}

async fn long_term_archive_invocation_query_for_range(
    pool: &Pool<Sqlite>,
) -> Result<LongTermArchiveInvocationRangeQueries> {
    let query = long_term_archive_invocation_query_parts(pool).await?;
    let canonical = format!(
        "{} WHERE {} AND instr(occurred_at, 'T') = 0 AND occurred_at >= ?1 AND occurred_at < ?2",
        query.select, query.terminal_filter
    );
    let crossing_text = format!(
        "{} WHERE {} AND occurred_at >= ?1 AND occurred_at < ?2 AND CASE WHEN instr(occurred_at, 'T') = 0 AND {} IS NOT NULL AND {} > 0 THEN julianday(occurred_at) + {} / 86400000.0 END >= julianday(?2)",
        query.select,
        query.terminal_filter,
        query.t_total_ms_column,
        query.t_total_ms_column,
        query.t_total_ms_column,
    );
    let rfc3339_epoch = long_term_rfc3339_whole_epoch_seconds_sql("occurred_at");
    let rfc3339_reaches_range_start =
        long_term_rfc3339_reaches_epoch_sql("occurred_at", &query.t_total_ms_column, "?3");
    let rfc3339 = format!(
        "{} WHERE {} AND instr(occurred_at, 'T') > 0 AND occurred_at >= ?1 AND occurred_at < ?2 AND {} < ?4 AND ({} >= ?3 OR ({} IS NOT NULL AND {} > 0 AND {}))",
        query.select,
        query.terminal_filter,
        rfc3339_epoch,
        rfc3339_epoch,
        query.t_total_ms_column,
        query.t_total_ms_column,
        rfc3339_reaches_range_start,
    );
    Ok(LongTermArchiveInvocationRangeQueries {
        canonical,
        crossing_text,
        rfc3339,
        parts: query,
    })
}

fn long_term_projection_update_compatibility_duration(
    maximum: &mut Option<f64>,
    duration_ms: Option<f64>,
) {
    let Some(duration_ms) = duration_ms.filter(|value| *value > 0.0) else {
        return;
    };
    if maximum.is_none_or(|current| duration_ms > current) {
        *maximum = Some(duration_ms);
    }
}

async fn inspect_long_term_archive_compatibility(
    pool: &Pool<Sqlite>,
    query: &LongTermArchiveInvocationQueryParts,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<LongTermArchiveCompatibility> {
    // Page by raw primary-key range rather than terminal rows. An archive can contain an
    // arbitrarily long pending prefix, so filtering inside SQL would make one nominal 512-row
    // page scan the full file before it can observe cancellation.
    let first_compatibility_rows = format!(
        "SELECT id, occurred_at, {} AS status, {} AS t_total_ms FROM codex_invocations ORDER BY id ASC LIMIT ?1",
        query.status_column, query.t_total_ms_column,
    );
    let next_compatibility_rows = format!(
        "SELECT id, occurred_at, {} AS status, {} AS t_total_ms FROM codex_invocations WHERE id > ?1 ORDER BY id ASC LIMIT ?2",
        query.status_column, query.t_total_ms_column,
    );
    let mut cursor = None;
    let mut legacy_max_duration_ms = None;
    let mut legacy_min_occurred_at = None;
    let mut rfc3339_max_duration_ms = None;
    let mut rfc3339_min_occurred_at = None;
    loop {
        control.check()?;
        let rows = if let Some(cursor) = cursor {
            sqlx::query_as::<_, LongTermArchiveCompatibilityRow>(&next_compatibility_rows)
                .bind(cursor)
                .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
                .fetch_all(pool)
                .await?
        } else {
            sqlx::query_as::<_, LongTermArchiveCompatibilityRow>(&first_compatibility_rows)
                .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
                .fetch_all(pool)
                .await?
        };
        let row_count = rows.len();
        for row in rows {
            cursor = Some(row.id);
            if row.status.as_deref().is_some_and(|status| {
                let status = status.trim();
                status.eq_ignore_ascii_case("running") || status.eq_ignore_ascii_case("pending")
            }) {
                continue;
            }
            if row.occurred_at.contains('T') {
                rfc3339_min_occurred_at = Some(
                    rfc3339_min_occurred_at.map_or(row.occurred_at.clone(), |current: String| {
                        current.min(row.occurred_at.clone())
                    }),
                );
                long_term_projection_update_compatibility_duration(
                    &mut rfc3339_max_duration_ms,
                    row.t_total_ms,
                );
            } else if row.t_total_ms.is_some_and(|duration_ms| duration_ms > 0.0) {
                legacy_min_occurred_at = Some(
                    legacy_min_occurred_at.map_or(row.occurred_at.clone(), |current: String| {
                        current.min(row.occurred_at.clone())
                    }),
                );
                long_term_projection_update_compatibility_duration(
                    &mut legacy_max_duration_ms,
                    row.t_total_ms,
                );
            }
        }
        control.complete_archive_compatibility_batch();
        if row_count < LONG_TERM_PROJECTION_WRITE_BATCH_ROWS {
            break;
        }
    }
    Ok(LongTermArchiveCompatibility {
        has_legacy_crossing: legacy_max_duration_ms.is_some(),
        legacy_max_duration_ms,
        legacy_min_occurred_at,
        has_rfc3339: rfc3339_min_occurred_at.is_some(),
        rfc3339_max_duration_ms,
        rfc3339_min_occurred_at,
    })
}

async fn load_long_term_archive_compatibility(
    pool: &Pool<Sqlite>,
    file_path: &str,
    archive_sha256: &str,
    file_fingerprint: &str,
) -> Result<Option<LongTermArchiveCompatibility>> {
    let row = sqlx::query_as::<_, (i64, Option<f64>, Option<String>, i64, Option<f64>, Option<String>)>(
        "SELECT has_legacy_crossing, legacy_max_duration_ms, legacy_min_occurred_at, has_rfc3339, rfc3339_max_duration_ms, rfc3339_min_occurred_at FROM long_term_projection_archive_compatibility WHERE file_path = ?1 AND archive_sha256 = ?2 AND file_fingerprint = ?3",
    )
    .bind(file_path)
    .bind(archive_sha256)
    .bind(file_fingerprint)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(
        |(
            has_legacy_crossing,
            legacy_max_duration_ms,
            legacy_min_occurred_at,
            has_rfc3339,
            rfc3339_max_duration_ms,
            rfc3339_min_occurred_at,
        )| {
            if has_legacy_crossing != 0
                && (legacy_max_duration_ms.is_none() || legacy_min_occurred_at.is_none())
                || has_rfc3339 != 0
                    && (rfc3339_max_duration_ms.is_none() || rfc3339_min_occurred_at.is_none())
            {
                // This cache entry predates the bounded crossing metadata. Reinspect the
                // immutable archive once rather than retaining its old prefix scan.
                None
            } else {
                Some(LongTermArchiveCompatibility {
                    has_legacy_crossing: has_legacy_crossing != 0,
                    legacy_max_duration_ms,
                    legacy_min_occurred_at,
                    has_rfc3339: has_rfc3339 != 0,
                    rfc3339_max_duration_ms,
                    rfc3339_min_occurred_at,
                })
            }
        },
    ))
}

async fn load_long_term_archive_sha256(
    pool: &Pool<Sqlite>,
    file_path: &str,
) -> Result<Option<String>> {
    load_long_term_archive_sha256_for_dataset(pool, "codex_invocations", file_path).await
}

async fn load_long_term_archive_sha256_for_dataset(
    pool: &Pool<Sqlite>,
    dataset: &str,
    file_path: &str,
) -> Result<Option<String>> {
    sqlx::query_scalar(
        "SELECT sha256 FROM archive_batches WHERE dataset = ?1 AND status = 'completed' AND file_path = ?2 ORDER BY id DESC LIMIT 1",
    )
    .bind(dataset)
    .bind(file_path)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn persist_long_term_archive_compatibility(
    pool: &Pool<Sqlite>,
    file_path: &str,
    archive_sha256: &str,
    file_fingerprint: &str,
    compatibility: LongTermArchiveCompatibility,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let (mut transaction, permit) = control.begin(pool).await?;
    sqlx::query(
        "INSERT INTO long_term_projection_archive_compatibility (file_path, archive_sha256, file_fingerprint, has_legacy_crossing, legacy_max_duration_ms, legacy_min_occurred_at, has_rfc3339, rfc3339_max_duration_ms, rfc3339_min_occurred_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT(file_path) DO UPDATE SET archive_sha256 = excluded.archive_sha256, file_fingerprint = excluded.file_fingerprint, has_legacy_crossing = excluded.has_legacy_crossing, legacy_max_duration_ms = excluded.legacy_max_duration_ms, legacy_min_occurred_at = excluded.legacy_min_occurred_at, has_rfc3339 = excluded.has_rfc3339, rfc3339_max_duration_ms = excluded.rfc3339_max_duration_ms, rfc3339_min_occurred_at = excluded.rfc3339_min_occurred_at, updated_at = datetime('now')",
    )
    .bind(file_path)
    .bind(archive_sha256)
    .bind(file_fingerprint)
    .bind(compatibility.has_legacy_crossing)
    .bind(compatibility.legacy_max_duration_ms)
    .bind(&compatibility.legacy_min_occurred_at)
    .bind(compatibility.has_rfc3339)
    .bind(compatibility.rfc3339_max_duration_ms)
    .bind(&compatibility.rfc3339_min_occurred_at)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await
}

async fn load_or_inspect_long_term_archive_compatibility(
    pool: &Pool<Sqlite>,
    archive_pool: &Pool<Sqlite>,
    file_path: &str,
    file_fingerprint: &str,
    query: &LongTermArchiveInvocationQueryParts,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<LongTermArchiveCompatibility> {
    let archive_sha256 = load_long_term_archive_sha256(pool, file_path).await?;
    if let Some(archive_sha256) = archive_sha256.as_deref()
        && let Some(compatibility) =
            load_long_term_archive_compatibility(pool, file_path, archive_sha256, file_fingerprint)
                .await?
    {
        return Ok(compatibility);
    }

    // Old archives can contain legacy or RFC3339 timestamps. Inspect once, then retain the
    // opened archive's checksum-and-file identity so ordinary canonical repairs stay range-seeked.
    let compatibility =
        inspect_long_term_archive_compatibility(archive_pool, query, control).await?;
    if let Some(archive_sha256) = archive_sha256.as_deref() {
        persist_long_term_archive_compatibility(
            pool,
            file_path,
            archive_sha256,
            file_fingerprint,
            compatibility.clone(),
            control,
        )
        .await?;
    }
    Ok(compatibility)
}

fn long_term_archive_file_fingerprint(file_path: &str) -> Result<String> {
    // Archive manifests are updated separately from file replacement. Hash the opened source
    // bytes rather than its mutable metadata so a stale manifest cannot reuse old capability.
    let mut file = std::fs::File::open(file_path).with_context(|| {
        format!("failed to open long-term archive {file_path} for fingerprinting")
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let bytes_read = std::io::Read::read(&mut file, &mut buffer)
            .with_context(|| format!("failed to fingerprint long-term archive {file_path}"))?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn long_term_archive_scan_identity_matches_manifest(
    scanned_sha256: &str,
    current_file_sha256: Option<&str>,
    manifest_sha256: Option<&str>,
) -> bool {
    current_file_sha256 == Some(scanned_sha256) && manifest_sha256 == Some(scanned_sha256)
}

fn long_term_archive_file_identity(file_path: &str) -> Result<String> {
    let metadata = std::fs::metadata(file_path)
        .with_context(|| format!("failed to stat long-term archive {file_path}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(format!(
            "unix:{}:{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.len(),
            metadata.mtime(),
            metadata.mtime_nsec()
        ))
    }
    #[cfg(not(unix))]
    {
        let modified_at = metadata
            .modified()
            .with_context(|| format!("failed to read long-term archive timestamp {file_path}"))?
            .duration_since(std::time::UNIX_EPOCH)
            .with_context(|| {
                format!("long-term archive timestamp predates the epoch {file_path}")
            })?;
        Ok(format!(
            "portable:{}:{}:{}",
            metadata.len(),
            modified_at.as_secs(),
            modified_at.subsec_nanos()
        ))
    }
}

async fn ensure_long_term_archive_source_identity(
    pool: &Pool<Sqlite>,
    dataset: &str,
    file_path: &str,
    expected_sha256: &str,
) -> Result<()> {
    let file_sha256 = crate::maintenance::sha256_hex_file(std::path::Path::new(file_path))?;
    let manifest_sha256 =
        load_long_term_archive_sha256_for_dataset(pool, dataset, file_path).await?;
    if long_term_archive_scan_identity_matches_manifest(
        expected_sha256,
        Some(&file_sha256),
        manifest_sha256.as_deref(),
    ) {
        Ok(())
    } else {
        bail!(
            "long-term archive source identity does not match its completed manifest: {file_path}"
        )
    }
}

async fn long_term_archive_pool_fingerprint(pool: &Pool<Sqlite>) -> Result<String> {
    let databases = sqlx::query_as::<_, (i64, String, String)>("PRAGMA database_list")
        .fetch_all(pool)
        .await
        .context("failed to resolve opened long-term archive database")?;
    let file_path = databases
        .into_iter()
        .find_map(|(_, name, file_path)| (name == "main").then_some(file_path))
        .filter(|file_path| !file_path.is_empty())
        .context("opened long-term archive does not expose a main database path")?;
    long_term_archive_file_fingerprint(&file_path)
}

fn long_term_archive_legacy_crossing_start(
    start: &chrono::DateTime<chrono_tz::Tz>,
    max_duration_ms: f64,
) -> Option<String> {
    if !max_duration_ms.is_finite() || max_duration_ms <= 0.0 {
        return None;
    }
    let max_duration_ms = max_duration_ms.ceil();
    if max_duration_ms > (i64::MAX - 1_000) as f64 {
        return None;
    }
    start
        .checked_sub_signed(ChronoDuration::milliseconds(max_duration_ms as i64 + 1_000))
        .map(|value| value.format("%Y-%m-%d %H:%M:%S").to_string())
}

fn long_term_rfc3339_text_bounds(
    start: chrono::DateTime<chrono_tz::Tz>,
    end: chrono::DateTime<chrono_tz::Tz>,
    compatibility: &LongTermRfc3339Compatibility,
) -> (String, String) {
    // RFC3339 input is accepted through -14:00 to +14:00. Relative to the +08:00 reporting
    // zone, the raw text for an instant can therefore be twenty-two hours earlier than its local
    // reporting time; leave one extra second for the exclusive lower boundary.
    const LONG_TERM_RFC3339_TEXT_LOWER_OFFSET_SECONDS: i64 = 22 * 60 * 60 + 1;
    let lower = compatibility
        .max_duration_ms
        .filter(|duration_ms| duration_ms.is_finite() && *duration_ms > 0.0)
        .and_then(|duration_ms| {
            let seconds = (duration_ms / 1000.0).ceil();
            (seconds <= (i64::MAX - LONG_TERM_RFC3339_TEXT_LOWER_OFFSET_SECONDS) as f64)
                .then_some(seconds as i64)
        })
        .and_then(|seconds| {
            start.checked_sub_signed(ChronoDuration::seconds(
                seconds + LONG_TERM_RFC3339_TEXT_LOWER_OFFSET_SECONDS,
            ))
        })
        .map(|value| value.format("%Y-%m-%dT%H:%M:%S").to_string())
        // Zero-duration rows cannot cross into the target date. Keep their candidate seek to the
        // RFC3339 offset window instead of reopening the range at the archive's first old row.
        .unwrap_or_else(|| {
            start
                .checked_sub_signed(ChronoDuration::seconds(
                    LONG_TERM_RFC3339_TEXT_LOWER_OFFSET_SECONDS,
                ))
                .unwrap_or(start)
                .format("%Y-%m-%dT%H:%M:%S")
                .to_string()
        });
    let upper = end
        .checked_add_signed(ChronoDuration::hours(15))
        .unwrap_or(end)
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string();
    (lower, upper)
}

async fn load_long_term_archive_invocation_rows_for_range(
    pool: &Pool<Sqlite>,
    queries: &LongTermArchiveInvocationRangeQueries,
    compatibility: LongTermArchiveCompatibility,
    start: chrono::DateTime<chrono_tz::Tz>,
    end: chrono::DateTime<chrono_tz::Tz>,
) -> Result<Vec<LongTermInvocationRow>> {
    let start_text = start.format("%Y-%m-%d %H:%M:%S").to_string();
    let end_text = end.format("%Y-%m-%d %H:%M:%S").to_string();
    let mut rows = sqlx::query_as::<_, LongTermInvocationRow>(&queries.canonical)
        .bind(&start_text)
        .bind(&end_text)
        .fetch_all(pool)
        .await?;
    if compatibility.has_legacy_crossing {
        let crossing_start = compatibility
            .legacy_max_duration_ms
            .and_then(|max_duration_ms| {
                long_term_archive_legacy_crossing_start(&start, max_duration_ms)
            })
            .or_else(|| compatibility.legacy_min_occurred_at.clone())
            .context("long-term archive legacy compatibility cache is missing a bounded start")?;
        let crossing_rows = sqlx::query_as::<_, LongTermInvocationRow>(&queries.crossing_text)
            .bind(crossing_start)
            .bind(&start_text)
            .fetch_all(pool)
            .await?;
        rows.extend(crossing_rows);
    }
    if compatibility.has_rfc3339 {
        let rfc3339_compatibility = LongTermRfc3339Compatibility {
            max_duration_ms: compatibility.rfc3339_max_duration_ms,
        };
        let (rfc3339_lower, rfc3339_upper) =
            long_term_rfc3339_text_bounds(start, end, &rfc3339_compatibility);
        rows.extend(
            sqlx::query_as::<_, LongTermInvocationRow>(&queries.rfc3339)
                .bind(rfc3339_lower)
                .bind(rfc3339_upper)
                .bind(start.timestamp())
                .bind(end.timestamp())
                .fetch_all(pool)
                .await?,
        );
    }
    // The canonical, legacy-crossing, and RFC3339 queries each preserve their own seek order.
    // Their union has no single text order across timestamp encodings, so retain a stable archive
    // row order without reinterpreting timestamps after the sargable range seeks.
    rows.sort_by_key(|row| row.id);
    Ok(rows)
}

fn long_term_rfc3339_whole_second_sql(value: &str) -> String {
    // SQLite normalizes RFC3339 fractions to milliseconds before evaluating `strftime` or
    // `julianday`. Strip the fraction before finding the whole second, then retain its original
    // digits so both exact boundaries and sub-millisecond values preserve their true ordering.
    let fraction_tail = format!("substr({value}, 21)");
    let fraction_end = format!(
        "CASE WHEN instr({fraction_tail}, 'Z') > 0 THEN instr({fraction_tail}, 'Z') - 1 WHEN instr({fraction_tail}, '+') > 0 THEN instr({fraction_tail}, '+') - 1 WHEN instr({fraction_tail}, '-') > 0 THEN instr({fraction_tail}, '-') - 1 ELSE length({value}) - 20 END"
    );
    let whole_second = format!(
        "CASE WHEN substr({value}, 20, 1) = '.' THEN substr({value}, 1, 19) || substr({value}, 21 + ({fraction_end})) ELSE {value} END"
    );
    whole_second
}

fn long_term_rfc3339_fraction_nanos_sql(value: &str) -> String {
    let fraction_tail = format!("substr({value}, 21)");
    let fraction_end = format!(
        "CASE WHEN instr({fraction_tail}, 'Z') > 0 THEN instr({fraction_tail}, 'Z') - 1 WHEN instr({fraction_tail}, '+') > 0 THEN instr({fraction_tail}, '+') - 1 WHEN instr({fraction_tail}, '-') > 0 THEN instr({fraction_tail}, '-') - 1 ELSE length({value}) - 20 END"
    );
    format!(
        "CASE WHEN substr({value}, 20, 1) = '.' THEN CAST(substr(substr({value}, 21, {fraction_end}) || '000000000', 1, 9) AS INTEGER) ELSE 0 END"
    )
}

fn long_term_rfc3339_whole_epoch_seconds_sql(value: &str) -> String {
    format!(
        "CAST(strftime('%s', {}) AS INTEGER)",
        long_term_rfc3339_whole_second_sql(value)
    )
}

fn long_term_rfc3339_elapsed_nanos_sql(duration_ms: &str) -> String {
    // Timestamps retain the first nine fractional digits as nanoseconds. The elapsed value is a
    // persisted SQLite REAL, so normalize only that bounded duration before comparing it with the
    // exact timestamp digits; never turn the RFC3339 fraction itself into a REAL.
    format!("CAST(ROUND(MAX(COALESCE({duration_ms}, 0), 0) * 1000000.0) AS INTEGER)")
}

fn long_term_rfc3339_fractional_carry_sql(value: &str, duration_ms: &str) -> String {
    let fraction_nanos = long_term_rfc3339_fraction_nanos_sql(value);
    let elapsed_nanos = long_term_rfc3339_elapsed_nanos_sql(duration_ms);
    let elapsed_whole_seconds = format!("CAST(({elapsed_nanos}) / 1000000000 AS INTEGER)");
    let elapsed_fraction_nanos =
        format!("(({elapsed_nanos}) - ({elapsed_whole_seconds}) * 1000000000)");
    format!(
        "CASE WHEN ({fraction_nanos}) + ({elapsed_fraction_nanos}) >= 1000000000 THEN 1 ELSE 0 END"
    )
}

fn long_term_rfc3339_reaches_epoch_sql(value: &str, duration_ms: &str, epoch: &str) -> String {
    let whole_epoch = long_term_rfc3339_whole_epoch_seconds_sql(value);
    let elapsed_nanos = long_term_rfc3339_elapsed_nanos_sql(duration_ms);
    let elapsed_whole_seconds = format!("CAST(({elapsed_nanos}) / 1000000000 AS INTEGER)");
    let fractional_carry = long_term_rfc3339_fractional_carry_sql(value, duration_ms);
    format!("(({whole_epoch}) + ({elapsed_whole_seconds}) + ({fractional_carry}) >= {epoch})")
}

fn long_term_rfc3339_shanghai_date_sql(value: &str, duration_ms: Option<&str>) -> String {
    // `date(..., 'unixepoch')` normalizes fractional seconds to milliseconds. Keep the raw
    // fractional tail outside date arithmetic, and carry it into the next exact second only
    // when a positive duration crosses that second.
    let whole_epoch = long_term_rfc3339_whole_epoch_seconds_sql(value);
    let elapsed_seconds = duration_ms.unwrap_or("0");
    let elapsed_nanos = long_term_rfc3339_elapsed_nanos_sql(elapsed_seconds);
    let elapsed_whole_seconds = format!("CAST(({elapsed_nanos}) / 1000000000 AS INTEGER)");
    let fractional_carry = long_term_rfc3339_fractional_carry_sql(value, elapsed_seconds);
    format!(
        "date(({whole_epoch}) + ({elapsed_whole_seconds}) + ({fractional_carry}), 'unixepoch', '+8 hours')"
    )
}

async fn long_term_archive_invocation_query_parts(
    pool: &Pool<Sqlite>,
) -> Result<LongTermArchiveInvocationQueryParts> {
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
    let select = format!(
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
        "#,
        invoke_id = select("invoke_id"),
        status = select("status"),
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
    );
    Ok(LongTermArchiveInvocationQueryParts {
        select,
        terminal_filter: format!(
            "LOWER(TRIM(COALESCE({status_column}, ''))) NOT IN ('running', 'pending')"
        ),
        status_column: status_column.to_string(),
        t_total_ms_column: t_total_ms_column.to_string(),
    })
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
            source_identity TEXT,
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
    if !replay_columns.contains("source_identity") {
        sqlx::query("ALTER TABLE hourly_rollup_archive_replay ADD COLUMN source_identity TEXT")
            .execute(pool)
            .await
            .context("failed to add archive file identity to replay markers")?;
    }
    ensure_long_term_projection_schema(pool).await?;
    ensure_long_term_projection_source_indexes(pool).await?;
    ensure_long_term_projection_correction_trigger(pool).await?;
    ensure_long_term_projection_archive_trigger(pool).await?;
    Ok(())
}

const LONG_TERM_PROJECTION_CONSUMER: &str = "long_term_v1";
const LONG_TERM_PROJECTION_FLUSH_INTERVAL: Duration = Duration::from_secs(60);
const LONG_TERM_PROJECTION_REPAIR_INTERVAL: Duration = Duration::from_secs(5 * 60);
const LONG_TERM_PROJECTION_DAILY_VERIFY_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const LONG_TERM_PROJECTION_MAX_BUCKETS_PER_FLUSH: i64 = 1;
const LONG_TERM_PROJECTION_MAX_EVENTS_PER_FLUSH: i64 = 2_000;
const LONG_TERM_PROJECTION_WRITE_BATCH_ROWS: usize = 512;
const LONG_TERM_PROJECTION_ADMISSION_WAIT: Duration = Duration::from_millis(250);
const LONG_TERM_PROJECTION_TRANSACTION_WAIT: Duration = Duration::from_millis(250);
// An incremental publication persists canonical interval state, rollups, its cursor, and status
// atomically. Keep every mutation in that one short transaction below the shared write limit.
const LONG_TERM_PROJECTION_INCREMENTAL_METADATA_ROWS: usize = 2;
const LONG_TERM_PROJECTION_INCREMENTAL_MUTATION_ROWS: usize =
    LONG_TERM_PROJECTION_WRITE_BATCH_ROWS - LONG_TERM_PROJECTION_INCREMENTAL_METADATA_ROWS;
// A rebuild segment updates canonical state, membership, and a suppression row. Its first
// transaction also updates the bucket marker, so leave one row of headroom.
const LONG_TERM_PROJECTION_REBUILD_SEGMENT_ROWS: usize =
    (LONG_TERM_PROJECTION_WRITE_BATCH_ROWS - 1) / 3;
// Staging more dates than this would make retry bookkeeping needlessly large. Publication is a
// single token transaction; each already-published date is released separately afterward.
const LONG_TERM_PROJECTION_REBUILD_PUBLICATION_DATES: usize =
    (LONG_TERM_PROJECTION_WRITE_BATCH_ROWS - 2) / 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LongTermProjectionFlushOutcome {
    Completed,
    DeferredByPressure { retry_at: Option<Instant> },
}

#[derive(Debug)]
struct LongTermProjectionWriteControl<'a> {
    shutdown: Option<&'a CancellationToken>,
    gate: Option<&'a crate::db_pressure::DbPressureGate>,
    #[cfg(test)]
    committed_batches: Option<(&'a AtomicUsize, usize)>,
    #[cfg(test)]
    cancel_after_commit: Option<(&'a CancellationToken, &'a AtomicUsize, usize)>,
    #[cfg(test)]
    stop_after_rebuild_chunk: Option<&'a CancellationToken>,
    #[cfg(test)]
    stop_after_refresh_publication: Option<&'a CancellationToken>,
    #[cfg(test)]
    stop_after_completed_integrity_repairs: Option<&'a CancellationToken>,
    #[cfg(test)]
    stop_after_backup_cleanup_marker: Option<&'a CancellationToken>,
    #[cfg(test)]
    stop_after_archive_compatibility_batch: Option<&'a CancellationToken>,
}

impl<'a> LongTermProjectionWriteControl<'a> {
    fn unrestricted() -> Self {
        Self {
            shutdown: None,
            gate: None,
            #[cfg(test)]
            committed_batches: None,
            #[cfg(test)]
            cancel_after_commit: None,
            #[cfg(test)]
            stop_after_rebuild_chunk: None,
            #[cfg(test)]
            stop_after_refresh_publication: None,
            #[cfg(test)]
            stop_after_completed_integrity_repairs: None,
            #[cfg(test)]
            stop_after_backup_cleanup_marker: None,
            #[cfg(test)]
            stop_after_archive_compatibility_batch: None,
        }
    }

    fn background(
        shutdown: &'a CancellationToken,
        gate: &'a crate::db_pressure::DbPressureGate,
    ) -> Self {
        Self {
            shutdown: Some(shutdown),
            gate: Some(gate),
            #[cfg(test)]
            committed_batches: None,
            #[cfg(test)]
            cancel_after_commit: None,
            #[cfg(test)]
            stop_after_rebuild_chunk: None,
            #[cfg(test)]
            stop_after_refresh_publication: None,
            #[cfg(test)]
            stop_after_completed_integrity_repairs: None,
            #[cfg(test)]
            stop_after_backup_cleanup_marker: None,
            #[cfg(test)]
            stop_after_archive_compatibility_batch: None,
        }
    }

    #[cfg(test)]
    fn stopping_after(
        shutdown: &'a CancellationToken,
        gate: &'a crate::db_pressure::DbPressureGate,
        committed_batches: &'a AtomicUsize,
        limit: usize,
    ) -> Self {
        Self {
            shutdown: Some(shutdown),
            gate: Some(gate),
            committed_batches: Some((committed_batches, limit)),
            cancel_after_commit: None,
            stop_after_rebuild_chunk: None,
            stop_after_refresh_publication: None,
            stop_after_completed_integrity_repairs: None,
            stop_after_backup_cleanup_marker: None,
            stop_after_archive_compatibility_batch: None,
        }
    }

    #[cfg(test)]
    fn cancelling_after(
        shutdown: &'a CancellationToken,
        gate: &'a crate::db_pressure::DbPressureGate,
        committed_batches: &'a AtomicUsize,
        limit: usize,
    ) -> Self {
        Self {
            shutdown: Some(shutdown),
            gate: Some(gate),
            committed_batches: None,
            cancel_after_commit: Some((shutdown, committed_batches, limit)),
            stop_after_rebuild_chunk: None,
            stop_after_refresh_publication: None,
            stop_after_completed_integrity_repairs: None,
            stop_after_backup_cleanup_marker: None,
            stop_after_archive_compatibility_batch: None,
        }
    }

    #[cfg(test)]
    fn stopping_after_rebuild_chunk(
        shutdown: &'a CancellationToken,
        gate: &'a crate::db_pressure::DbPressureGate,
    ) -> Self {
        Self {
            shutdown: Some(shutdown),
            gate: Some(gate),
            committed_batches: None,
            cancel_after_commit: None,
            stop_after_rebuild_chunk: Some(shutdown),
            stop_after_refresh_publication: None,
            stop_after_completed_integrity_repairs: None,
            stop_after_backup_cleanup_marker: None,
            stop_after_archive_compatibility_batch: None,
        }
    }

    #[cfg(test)]
    fn stopping_after_refresh_publication(
        shutdown: &'a CancellationToken,
        gate: &'a crate::db_pressure::DbPressureGate,
    ) -> Self {
        Self {
            shutdown: Some(shutdown),
            gate: Some(gate),
            committed_batches: None,
            cancel_after_commit: None,
            stop_after_rebuild_chunk: None,
            stop_after_refresh_publication: Some(shutdown),
            stop_after_completed_integrity_repairs: None,
            stop_after_backup_cleanup_marker: None,
            stop_after_archive_compatibility_batch: None,
        }
    }

    #[cfg(test)]
    fn stopping_after_backup_cleanup_marker(
        shutdown: &'a CancellationToken,
        gate: &'a crate::db_pressure::DbPressureGate,
    ) -> Self {
        Self {
            shutdown: Some(shutdown),
            gate: Some(gate),
            committed_batches: None,
            cancel_after_commit: None,
            stop_after_rebuild_chunk: None,
            stop_after_refresh_publication: None,
            stop_after_completed_integrity_repairs: None,
            stop_after_backup_cleanup_marker: Some(shutdown),
            stop_after_archive_compatibility_batch: None,
        }
    }

    #[cfg(test)]
    fn stopping_after_archive_compatibility_batch(
        shutdown: &'a CancellationToken,
        gate: &'a crate::db_pressure::DbPressureGate,
    ) -> Self {
        Self {
            shutdown: Some(shutdown),
            gate: Some(gate),
            committed_batches: None,
            cancel_after_commit: None,
            stop_after_rebuild_chunk: None,
            stop_after_refresh_publication: None,
            stop_after_completed_integrity_repairs: None,
            stop_after_backup_cleanup_marker: None,
            stop_after_archive_compatibility_batch: Some(shutdown),
        }
    }

    fn complete_rebuild_chunk(&self) {
        #[cfg(test)]
        if let Some(shutdown) = self.stop_after_rebuild_chunk {
            shutdown.cancel();
        }
    }

    fn complete_refresh_publication(&self) {
        #[cfg(test)]
        if let Some(shutdown) = self.stop_after_refresh_publication {
            shutdown.cancel();
        }
    }

    #[cfg(test)]
    fn stopping_after_completed_integrity_repairs(
        shutdown: &'a CancellationToken,
        gate: &'a crate::db_pressure::DbPressureGate,
    ) -> Self {
        Self {
            shutdown: Some(shutdown),
            gate: Some(gate),
            committed_batches: None,
            cancel_after_commit: None,
            stop_after_rebuild_chunk: None,
            stop_after_refresh_publication: None,
            stop_after_completed_integrity_repairs: Some(shutdown),
            stop_after_backup_cleanup_marker: None,
            stop_after_archive_compatibility_batch: None,
        }
    }

    fn complete_integrity_repairs(&self) {
        #[cfg(test)]
        if let Some(shutdown) = self.stop_after_completed_integrity_repairs {
            shutdown.cancel();
        }
    }

    fn complete_backup_cleanup_marker(&self) {
        #[cfg(test)]
        if let Some(shutdown) = self.stop_after_backup_cleanup_marker {
            shutdown.cancel();
        }
    }

    fn complete_archive_compatibility_batch(&self) {
        #[cfg(test)]
        if let Some(shutdown) = self.stop_after_archive_compatibility_batch {
            shutdown.cancel();
        }
    }

    fn check(&self) -> Result<()> {
        if self.shutdown.is_some_and(CancellationToken::is_cancelled) {
            bail!("long-term projection write cancelled");
        }
        #[cfg(test)]
        if self
            .committed_batches
            .is_some_and(|(count, limit)| count.load(Ordering::Acquire) >= limit)
        {
            bail!("long-term projection write cancelled after committed batch");
        }
        Ok(())
    }

    async fn begin<'p>(
        &self,
        pool: &'p Pool<Sqlite>,
    ) -> Result<(
        sqlx::Transaction<'p, Sqlite>,
        Option<crate::db_pressure::DbBackgroundPermit>,
    )> {
        self.check()?;
        let permit = if let Some(gate) = self.gate {
            let result = if let Some(shutdown) = self.shutdown {
                tokio::select! {
                    _ = shutdown.cancelled() => bail!("long-term projection write cancelled"),
                    result = gate.begin_background_with_busy_wait(
                        "long_term_projection_write",
                        LONG_TERM_PROJECTION_ADMISSION_WAIT,
                    ) => result,
                }
            } else {
                gate.begin_background_with_busy_wait(
                    "long_term_projection_write",
                    LONG_TERM_PROJECTION_ADMISSION_WAIT,
                )
                .await
            };
            Some(result.map_err(|reason| {
                anyhow!("long-term projection write deferred by database pressure: {reason}")
            })?)
        } else {
            None
        };
        let transaction = if let Some(shutdown) = self.shutdown {
            tokio::select! {
                _ = shutdown.cancelled() => bail!("long-term projection write cancelled"),
                result = tokio::time::timeout(
                    LONG_TERM_PROJECTION_TRANSACTION_WAIT,
                    pool.begin_with("BEGIN IMMEDIATE"),
                ) => match result {
                    Ok(transaction) => transaction?,
                    Err(_) => {
                        if let Some(gate) = self.gate {
                            gate.record_pressure(
                                "long_term_projection_write",
                                "transaction_admission_timeout",
                            );
                        }
                        bail!("long-term projection write deferred by database pressure: transaction admission timed out");
                    }
                },
            }
        } else {
            pool.begin().await?
        };
        Ok((transaction, permit))
    }

    async fn commit(
        &self,
        transaction: sqlx::Transaction<'_, Sqlite>,
        permit: Option<crate::db_pressure::DbBackgroundPermit>,
    ) -> Result<()> {
        if let Some(shutdown) = self.shutdown {
            tokio::select! {
                _ = shutdown.cancelled() => bail!("long-term projection write cancelled"),
                result = tokio::time::timeout(LONG_TERM_PROJECTION_TRANSACTION_WAIT, transaction.commit()) => match result {
                    Ok(result) => result?,
                    Err(_) => {
                        if let Some(gate) = self.gate {
                            gate.record_pressure(
                                "long_term_projection_write",
                                "transaction_commit_timeout",
                            );
                        }
                        bail!("long-term projection write deferred by database pressure: transaction commit timed out");
                    }
                },
            }
        } else {
            transaction.commit().await?;
        }
        drop(permit);
        #[cfg(test)]
        if let Some((count, _)) = self.committed_batches {
            count.fetch_add(1, Ordering::AcqRel);
        }
        #[cfg(test)]
        if let Some((shutdown, count, limit)) = self.cancel_after_commit
            && count.fetch_add(1, Ordering::AcqRel) + 1 >= limit
        {
            shutdown.cancel();
        }
        Ok(())
    }
}

fn long_term_projection_write_is_deferred(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("long-term projection write deferred by database pressure")
        || message.contains("long-term projection write cancelled")
}

fn long_term_projection_write_is_pressure_deferred(error: &anyhow::Error) -> bool {
    error
        .to_string()
        .contains("long-term projection write deferred by database pressure")
}

async fn ensure_long_term_projection_source_indexes(pool: &Pool<Sqlite>) -> Result<()> {
    let invocation_table_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'codex_invocations')",
    )
    .fetch_one(pool)
    .await?
        != 0;
    if !invocation_table_exists {
        return Ok(());
    }
    let columns = load_sqlite_table_columns(pool, "codex_invocations").await?;
    if columns.contains("status") {
        // Terminal delta scans advance by id. Index the exact terminal predicate so a dense
        // running/pending prefix cannot turn a bounded incremental flush into a table scan.
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_codex_invocations_long_term_projection_terminal_id
            ON codex_invocations (id)
            WHERE LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')
            "#,
        )
        .execute(pool)
        .await
        .context("failed to ensure long-term projection terminal id index")?;
    }
    if !columns.contains("occurred_at") || !columns.contains("t_total_ms") {
        return Ok(());
    }
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_long_term_projection_text_end
        ON codex_invocations (
            CASE
                WHEN instr(occurred_at, 'T') = 0
                  AND t_total_ms IS NOT NULL
                  AND t_total_ms > 0
                THEN julianday(occurred_at) + t_total_ms / 86400000.0
            END
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection text end index")?;
    if columns.contains("status") {
        // RFC3339 rows are exceptional, but preserving them requires an index-backed candidate
        // range before the exact (and deliberately compatibility-safe) epoch predicate runs.
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_codex_invocations_long_term_projection_rfc3339_occurred_at
            ON codex_invocations (occurred_at)
            WHERE instr(occurred_at, 'T') > 0
              AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')
            "#,
        )
        .execute(pool)
        .await
        .context("failed to ensure long-term projection RFC3339 timestamp index")?;
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_codex_invocations_long_term_projection_rfc3339_duration
            ON codex_invocations (t_total_ms DESC)
            WHERE instr(occurred_at, 'T') > 0
              AND t_total_ms IS NOT NULL
              AND t_total_ms > 0
              AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')
            "#,
        )
        .execute(pool)
        .await
        .context("failed to ensure long-term projection RFC3339 duration index")?;
    }
    Ok(())
}

async fn ensure_long_term_projection_correction_trigger(pool: &Pool<Sqlite>) -> Result<()> {
    let invocation_table_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'codex_invocations')",
    )
    .fetch_one(pool)
    .await?;
    if invocation_table_exists == 0 {
        return Ok(());
    }

    // Corrections originate in several write-side workers. A normal terminal finalize moves an
    // in-flight record to a terminal state and is consumed through the projection cursor unless
    // a newer terminal row has already advanced that cursor past it. In that out-of-order case,
    // the cursor cannot revisit the row, so queue the affected date for an exact repair.
    sqlx::query("DROP TRIGGER IF EXISTS long_term_projection_invocation_correction")
        .execute(pool)
        .await?;
    let old_start_date = long_term_rfc3339_shanghai_date_sql("OLD.occurred_at", None);
    let old_end_date = long_term_rfc3339_shanghai_date_sql(
        "OLD.occurred_at",
        Some("MAX(COALESCE(OLD.t_total_ms, 0), 0)"),
    );
    let new_start_date = long_term_rfc3339_shanghai_date_sql("NEW.occurred_at", None);
    let new_end_date = long_term_rfc3339_shanghai_date_sql(
        "NEW.occurred_at",
        Some("MAX(COALESCE(NEW.t_total_ms, 0), 0)"),
    );
    sqlx::query(&format!(
        r#"
        CREATE TRIGGER IF NOT EXISTS long_term_projection_invocation_correction
        AFTER UPDATE OF
          source, status, occurred_at, model, payload,
          input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens,
          cost, t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms,
          t_upstream_ttfb_ms, t_upstream_stream_ms, error_message, failure_kind
        ON codex_invocations
        WHEN NOT (
          (
            LOWER(TRIM(COALESCE(OLD.status, ''))) IN ('running', 'pending')
            OR (
              LOWER(TRIM(COALESCE(OLD.status, ''))) = 'interrupted'
              AND LOWER(TRIM(COALESCE(OLD.failure_kind, ''))) = 'proxy_interrupted'
            )
          )
          AND LOWER(TRIM(COALESCE(NEW.status, ''))) NOT IN ('running', 'pending')
          AND NEW.id > COALESCE(
            (
              SELECT cursor_row_id
              FROM long_term_projection_state
              WHERE consumer = 'long_term_v1'
            ),
            0
          )
        )
        BEGIN
          INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason)
          WITH RECURSIVE affected_dates(bucket_date, end_date) AS (
            SELECT
              CASE WHEN instr(OLD.occurred_at, 'T') > 0
                THEN {old_start_date}
                ELSE date(OLD.occurred_at) END,
              CASE WHEN instr(OLD.occurred_at, 'T') > 0
                THEN {old_end_date}
                ELSE date(julianday(OLD.occurred_at) + MAX(COALESCE(OLD.t_total_ms, 0), 0) / 86400000.0) END
            WHERE OLD.occurred_at IS NOT NULL AND TRIM(OLD.occurred_at) <> ''
            UNION ALL
            SELECT
              CASE WHEN instr(NEW.occurred_at, 'T') > 0
                THEN {new_start_date}
                ELSE date(NEW.occurred_at) END,
              CASE WHEN instr(NEW.occurred_at, 'T') > 0
                THEN {new_end_date}
                ELSE date(julianday(NEW.occurred_at) + MAX(COALESCE(NEW.t_total_ms, 0), 0) / 86400000.0) END
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
            generation = long_term_projection_dirty_buckets.generation + 1,
            next_attempt_at = NULL,
            updated_at = datetime('now');
        END
        "#,
    ))
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

    let new_coverage_start_date =
        long_term_rfc3339_shanghai_date_sql("NEW.coverage_start_at", None);
    let new_coverage_end_date = long_term_rfc3339_shanghai_date_sql("NEW.coverage_end_at", None);
    let old_coverage_start_date =
        long_term_rfc3339_shanghai_date_sql("OLD.coverage_start_at", None);
    let old_coverage_end_date = long_term_rfc3339_shanghai_date_sql("OLD.coverage_end_at", None);

    sqlx::query(&format!(
        r#"
        CREATE TRIGGER IF NOT EXISTS long_term_projection_archive_insert
        AFTER INSERT ON archive_batches
        WHEN NEW.dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
          AND NEW.status = 'completed'
        BEGIN
          INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason)
          WITH RECURSIVE covered_dates(bucket_date, end_date) AS (
            SELECT
              COALESCE(CASE WHEN instr(NEW.coverage_start_at, 'T') > 0 THEN {new_coverage_start_date} ELSE date(NEW.coverage_start_at) END, date(NEW.month_key || '-01')),
              COALESCE(CASE WHEN instr(NEW.coverage_end_at, 'T') > 0 THEN {new_coverage_end_date} ELSE date(NEW.coverage_end_at) END, date(NEW.month_key || '-01', '+1 month', '-1 day'))
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
            generation = long_term_projection_dirty_buckets.generation + 1,
            next_attempt_at = NULL,
            updated_at = datetime('now');
        END
        "#,
    ))
    .execute(pool)
    .await
    .context("failed to ensure long-term projection archive insert trigger")?;

    sqlx::query(&format!(
        r#"
        CREATE TRIGGER IF NOT EXISTS long_term_projection_archive_update
        AFTER UPDATE OF dataset, status, file_path, sha256, coverage_start_at, coverage_end_at, historical_rollups_materialized_at ON archive_batches
        WHEN (
              NEW.dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
              AND NEW.status = 'completed'
            )
            OR (
              OLD.dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
              AND OLD.status = 'completed'
            )
        BEGIN
          INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason)
          WITH RECURSIVE coverage_ranges(start_date, end_date) AS (
            SELECT
              COALESCE(CASE WHEN instr(NEW.coverage_start_at, 'T') > 0 THEN {new_coverage_start_date} ELSE date(NEW.coverage_start_at) END, date(NEW.month_key || '-01')),
              COALESCE(CASE WHEN instr(NEW.coverage_end_at, 'T') > 0 THEN {new_coverage_end_date} ELSE date(NEW.coverage_end_at) END, date(NEW.month_key || '-01', '+1 month', '-1 day'))
            WHERE NEW.dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
              AND NEW.status = 'completed'
            UNION ALL
            SELECT
              COALESCE(CASE WHEN instr(OLD.coverage_start_at, 'T') > 0 THEN {old_coverage_start_date} ELSE date(OLD.coverage_start_at) END, date(OLD.month_key || '-01')),
              COALESCE(CASE WHEN instr(OLD.coverage_end_at, 'T') > 0 THEN {old_coverage_end_date} ELSE date(OLD.coverage_end_at) END, date(OLD.month_key || '-01', '+1 month', '-1 day'))
            WHERE OLD.dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
              AND OLD.status = 'completed'
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
            generation = long_term_projection_dirty_buckets.generation + 1,
            next_attempt_at = NULL,
            updated_at = datetime('now');
        END
        "#,
    ))
    .execute(pool)
    .await
    .context("failed to ensure long-term projection archive update trigger")?;
    Ok(())
}

pub(crate) async fn ensure_long_term_projection_account_trigger(pool: &Pool<Sqlite>) -> Result<()> {
    for trigger in [
        "long_term_projection_account_kind_update",
        "long_term_projection_account_delete",
    ] {
        sqlx::query(&format!("DROP TRIGGER IF EXISTS {trigger}"))
            .execute(pool)
            .await?;
    }
    for (trigger, event, account_id) in [
        (
            "long_term_projection_account_kind_update",
            "AFTER UPDATE OF kind ON pool_upstream_accounts WHEN OLD.kind IS NOT NEW.kind",
            "NEW.id",
        ),
        (
            "long_term_projection_account_delete",
            "AFTER DELETE ON pool_upstream_accounts",
            "OLD.id",
        ),
    ] {
        let occurred_at_start_date = long_term_rfc3339_shanghai_date_sql("inv.occurred_at", None);
        let occurred_at_end_date = long_term_rfc3339_shanghai_date_sql(
            "inv.occurred_at",
            Some("MAX(COALESCE(inv.t_total_ms, 0), 0)"),
        );
        let archive_coverage_start_date =
            long_term_rfc3339_shanghai_date_sql("archive.coverage_start_at", None);
        let archive_coverage_end_date =
            long_term_rfc3339_shanghai_date_sql("archive.coverage_end_at", None);
        let statement = format!(
            r#"
            CREATE TRIGGER IF NOT EXISTS {trigger}
            {event}
            BEGIN
              INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason)
              WITH RECURSIVE affected_dates(bucket_date, end_date) AS (
                SELECT
                  CASE WHEN instr(inv.occurred_at, 'T') > 0
                    THEN {occurred_at_start_date}
                    ELSE date(inv.occurred_at) END,
                  CASE WHEN instr(inv.occurred_at, 'T') > 0
                    THEN {occurred_at_end_date}
                    ELSE date(julianday(inv.occurred_at) + MAX(COALESCE(inv.t_total_ms, 0), 0) / 86400000.0) END
                FROM codex_invocations inv
                WHERE (CASE WHEN json_valid(inv.payload) THEN CAST(json_extract(inv.payload, '$.upstreamAccountId') AS INTEGER) END) = {account_id}
                   OR EXISTS (
                        SELECT 1 FROM pool_upstream_request_attempts attempt
                        WHERE attempt.invoke_id = inv.invoke_id
                          AND attempt.occurred_at = inv.occurred_at
                          AND attempt.upstream_account_id = {account_id}
                      )
                UNION ALL
                SELECT
                  COALESCE(CASE WHEN instr(archive.coverage_start_at, 'T') > 0
                    THEN {archive_coverage_start_date}
                    ELSE date(archive.coverage_start_at) END, date(archive.month_key || '-01')),
                  COALESCE(CASE WHEN instr(archive.coverage_end_at, 'T') > 0
                    THEN {archive_coverage_end_date}
                    ELSE date(archive.coverage_end_at) END, date(archive.month_key || '-01', '+1 month', '-1 day'))
                FROM archive_batches archive
                WHERE archive.dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
                  AND archive.status = 'completed'
                UNION ALL
                SELECT date(bucket_date, '+1 day'), end_date
                FROM affected_dates
                WHERE bucket_date < end_date
              )
              SELECT DISTINCT bucket_date, 'account_classification_changed'
              FROM affected_dates
              WHERE bucket_date IS NOT NULL
              ON CONFLICT(bucket_date) DO UPDATE SET
                repair_reason = excluded.repair_reason,
                generation = long_term_projection_dirty_buckets.generation + 1,
                next_attempt_at = NULL,
                updated_at = datetime('now');
            END
            "#,
        );
        sqlx::query(&statement).execute(pool).await?;
    }
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
    spawn_long_term_stats_backfill(
        state.pool.clone(),
        state.config.long_term_stats_hourly_retention_days,
        cancel.clone(),
    );
    tokio::spawn(async move {
        let pressure_gate = crate::db_pressure::global_db_pressure_gate();
        let mut pressure_eligibility_generation = pressure_gate.eligibility_generation();
        let mut pressure_retry_pending = false;
        let mut pressure_retry_at = None;
        let mut flush_ticker = interval(LONG_TERM_PROJECTION_FLUSH_INTERVAL);
        flush_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        flush_ticker.tick().await;
        let mut maintenance_ticker = interval(LONG_TERM_PROJECTION_FLUSH_INTERVAL);
        maintenance_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        maintenance_ticker.tick().await;
        let mut repair_ticker = interval(LONG_TERM_PROJECTION_REPAIR_INTERVAL);
        repair_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        repair_ticker.tick().await;
        let mut daily_verify_ticker = interval(LONG_TERM_PROJECTION_DAILY_VERIFY_INTERVAL);
        daily_verify_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        daily_verify_ticker.tick().await;

        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = wait_for_long_term_projection_pressure_retry(
                    pressure_gate,
                    pressure_eligibility_generation,
                    pressure_retry_at,
                ), if pressure_retry_pending => {
                    pressure_eligibility_generation = pressure_gate.eligibility_generation();
                    match flush_long_term_projection(&state, "pressure_eligible").await {
                        Ok(LongTermProjectionFlushOutcome::Completed) => {
                            pressure_retry_pending = false;
                            pressure_retry_at = None;
                        }
                        Ok(LongTermProjectionFlushOutcome::DeferredByPressure { retry_at }) => {
                            pressure_retry_at = retry_at;
                        }
                        Err(error) => {
                            pressure_retry_pending = false;
                            pressure_retry_at = None;
                            mark_long_term_projection_failure(&state, &error).await;
                            warn!(error = %error, projection = "long_term", trigger = "pressure_eligible", "long-term projection pressure retry failed");
                        }
                    }
                }
                _ = state.terminal_projection_hub.wait_for_persisted_work() => {
                    debug!(projection = "long_term", trigger = "terminal_p1_ack", "long-term projection marked dirty by terminal persistence");
                }
                _ = flush_ticker.tick() => {
                    let trigger = long_term_projection_terminal_flush_needed(&state)
                        .await
                        .then_some("terminal_deadline");
                    if let Some(trigger) = trigger {
                        pressure_eligibility_generation = pressure_gate.eligibility_generation();
                        match flush_long_term_projection(&state, trigger).await {
                            Ok(LongTermProjectionFlushOutcome::Completed) => {
                                pressure_retry_pending = false;
                                pressure_retry_at = None;
                            }
                            Ok(LongTermProjectionFlushOutcome::DeferredByPressure { retry_at }) => {
                                pressure_retry_pending = true;
                                pressure_retry_at = retry_at;
                            }
                            Err(error) => {
                                mark_long_term_projection_failure(&state, &error).await;
                                warn!(error = %error, projection = "long_term", trigger, "long-term projection flush failed");
                            }
                        }
                    } else {
                        debug!(projection = "long_term", trigger = "terminal_deadline", flush_outcome = "noop_suppressed", "skipping idle long-term projection flush");
                    }
                }
                _ = maintenance_ticker.tick() => {
                    match long_term_projection_maintenance_needed(
                        &state.pool,
                        state.config.long_term_stats_hourly_retention_days,
                    )
                    .await
                    {
                        Ok(true) => {
                            pressure_eligibility_generation = pressure_gate.eligibility_generation();
                            match flush_long_term_projection(&state, "maintenance_deadline").await {
                                Ok(LongTermProjectionFlushOutcome::Completed) => {
                                    pressure_retry_pending = false;
                                    pressure_retry_at = None;
                                }
                                Ok(LongTermProjectionFlushOutcome::DeferredByPressure { retry_at }) => {
                                    pressure_retry_pending = true;
                                    pressure_retry_at = retry_at;
                                }
                                Err(error) => {
                                    mark_long_term_projection_failure(&state, &error).await;
                                    warn!(error = %error, projection = "long_term", trigger = "maintenance_deadline", "long-term projection maintenance failed");
                                }
                            }
                        }
                        Ok(false) => {
                            debug!(projection = "long_term", trigger = "maintenance_deadline", flush_outcome = "noop_suppressed", "skipping idle long-term projection maintenance");
                        }
                        Err(error) => {
                            mark_long_term_projection_failure(&state, &error).await;
                            warn!(error = %error, projection = "long_term", trigger = "maintenance_deadline", "failed to inspect long-term projection maintenance work");
                        }
                    }
                }
                _ = repair_ticker.tick() => {
                    match long_term_projection_repair_needed(&state).await {
                        Ok(true) => {
                            pressure_eligibility_generation = pressure_gate.eligibility_generation();
                            match flush_long_term_projection(&state, "repair_deadline").await {
                                Ok(LongTermProjectionFlushOutcome::Completed) => {
                                    pressure_retry_pending = false;
                                    pressure_retry_at = None;
                                }
                                Ok(LongTermProjectionFlushOutcome::DeferredByPressure { retry_at }) => {
                                    pressure_retry_pending = true;
                                    pressure_retry_at = retry_at;
                                }
                                Err(error) => {
                                    mark_long_term_projection_failure(&state, &error).await;
                                    warn!(error = %error, projection = "long_term", trigger = "repair_deadline", "long-term projection repair failed");
                                }
                            }
                        }
                        Ok(false) => {
                            debug!(projection = "long_term", trigger = "repair_deadline", flush_outcome = "noop_suppressed", "skipping idle long-term repair");
                        }
                        Err(error) => {
                            mark_long_term_projection_failure(&state, &error).await;
                            warn!(error = %error, projection = "long_term", trigger = "repair_deadline", "failed to inspect long-term projection repair work");
                        }
                    }
                }
                _ = daily_verify_ticker.tick() => {
                    pressure_eligibility_generation = pressure_gate.eligibility_generation();
                    match flush_long_term_projection(&state, "daily_verify").await {
                        Ok(LongTermProjectionFlushOutcome::Completed) => {
                            pressure_retry_pending = false;
                            pressure_retry_at = None;
                        }
                        Ok(LongTermProjectionFlushOutcome::DeferredByPressure { retry_at }) => {
                            pressure_retry_pending = true;
                            pressure_retry_at = retry_at;
                        }
                        Err(error) => {
                            mark_long_term_projection_failure(&state, &error).await;
                            warn!(error = %error, projection = "long_term", trigger = "daily_verify", "long-term projection daily verification failed");
                        }
                    }
                }
            }
        }
    })
}

async fn long_term_projection_terminal_flush_needed(state: &AppState) -> bool {
    let has_persisted_work = state.terminal_projection_hub.has_persisted_work();
    let runtime = state.long_term_projection_runtime.lock().await;
    long_term_projection_terminal_flush_due(has_persisted_work, runtime.state.is_empty())
}

async fn long_term_projection_maintenance_needed(
    pool: &Pool<Sqlite>,
    retention_days: u64,
) -> Result<bool> {
    if sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM long_term_projection_intervals LIMIT 1)",
    )
    .fetch_one(pool)
    .await?
        != 0
    {
        return Ok(true);
    }

    let retention_start_date = long_term_projection_hourly_retention_start_date(retention_days);
    let retention_start_epoch = retention_start_date
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Shanghai.from_local_datetime(&value).single())
        .map(|value| value.timestamp())
        .context("invalid long-term projection hourly retention start")?;
    let retention_start_ms = retention_start_epoch * 1_000;
    for (statement, value) in [
        (
            "SELECT EXISTS(SELECT 1 FROM long_term_usage_hourly WHERE bucket_start_epoch < ?1 LIMIT 1)",
            retention_start_epoch,
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM long_term_projection_interval_state WHERE interval_end_ms < ?1 LIMIT 1)",
            retention_start_ms,
        ),
    ] {
        if sqlx::query_scalar::<_, i64>(statement)
            .bind(value)
            .fetch_one(pool)
            .await?
            != 0
        {
            return Ok(true);
        }
    }
    for table in [
        "long_term_projection_interval_suppressions",
        "long_term_projection_rebuild_members",
    ] {
        let statement = format!(
            "SELECT EXISTS(SELECT 1 FROM {table} metadata WHERE NOT EXISTS (SELECT 1 FROM long_term_projection_interval_state state WHERE state.invocation_row_id = metadata.invocation_row_id) LIMIT 1)"
        );
        if sqlx::query_scalar::<_, i64>(&statement)
            .fetch_one(pool)
            .await?
            != 0
        {
            return Ok(true);
        }
    }
    for statement in [
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM long_term_projection_daily_backups backup
            JOIN long_term_projection_bucket_state state
              ON state.bucket_date = backup.stats_date
             AND state.active_daily_backup_token IS NULL
             AND state.publication_token = 'cleanup:' || backup.rebuild_token
            WHERE NOT EXISTS (
                SELECT 1
                FROM long_term_projection_daily_backup_claims claim
                WHERE claim.bucket_date = state.bucket_date
                  AND claim.rebuild_token = backup.rebuild_token
            )
            LIMIT 1
        )
        "#,
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM long_term_projection_bucket_state state
            WHERE state.active_daily_backup_token IS NULL
              AND state.publication_token LIKE 'cleanup:%'
              AND NOT EXISTS (
                  SELECT 1
                  FROM long_term_projection_daily_backups backup
                  WHERE backup.rebuild_token = substr(state.publication_token, 9)
              )
            LIMIT 1
        )
        "#,
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM long_term_projection_bucket_state state
            JOIN long_term_projection_date_publications publication
              ON publication.publication_token = state.publication_token
            WHERE publication.published = 1
              AND state.active_daily_backup_token IS NOT NULL
            LIMIT 1
        )
        "#,
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM long_term_projection_date_publications publication
            WHERE NOT EXISTS (
                SELECT 1
                FROM long_term_projection_bucket_state state
                WHERE state.publication_token = publication.publication_token
            )
            LIMIT 1
        )
        "#,
    ] {
        if sqlx::query_scalar::<_, i64>(statement)
            .fetch_one(pool)
            .await?
            != 0
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn long_term_projection_terminal_flush_due(
    has_persisted_work: bool,
    runtime_state_is_empty: bool,
) -> bool {
    // A deferred date rebuild must not suppress the bounded terminal delta pass.
    // The pass can still advance any ready prefix while the repair ticker owns
    // the expensive rebuild deadline.
    has_persisted_work || runtime_state_is_empty
}

async fn long_term_projection_repair_needed(state: &AppState) -> Result<bool> {
    let has_persisted_work = state.terminal_projection_hub.has_persisted_work();
    let (runtime_state_is_empty, next_repair_at) = {
        let runtime = state.long_term_projection_runtime.lock().await;
        (runtime.state.is_empty(), runtime.next_repair_at)
    };
    let now = Instant::now();
    if long_term_projection_repair_due(
        has_persisted_work,
        false,
        runtime_state_is_empty,
        next_repair_at,
        now,
    ) {
        return Ok(true);
    }

    // Correction and archive triggers write durable dirty markers without touching the
    // in-memory runtime state. Probe them only on the bounded repair cadence.
    let has_due_dirty_bucket = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM long_term_projection_dirty_buckets WHERE next_attempt_at IS NULL OR datetime(next_attempt_at) <= datetime('now'))",
    )
    .fetch_one(&state.pool)
    .await?
        != 0;
    let daily_verify_due = long_term_projection_daily_verify_due(&state.pool).await?;
    Ok(long_term_projection_repair_due(
        has_persisted_work,
        has_due_dirty_bucket || daily_verify_due,
        runtime_state_is_empty,
        next_repair_at,
        now,
    ))
}

fn long_term_projection_repair_due(
    has_persisted_work: bool,
    has_due_dirty_bucket: bool,
    runtime_state_is_empty: bool,
    next_repair_at: Option<Instant>,
    now: Instant,
) -> bool {
    // A scheduled repair owns the expensive rebuild deadline. Persisted terminal
    // deltas still use the bounded terminal pass, but must not pull this rebuild
    // forward before the deadline.
    next_repair_at.map_or(
        has_persisted_work || has_due_dirty_bucket || runtime_state_is_empty,
        |retry_at| retry_at <= now,
    )
}

async fn defer_long_term_projection_terminal_repair(state: &AppState, defer_reason: &'static str) {
    let mut runtime = state.long_term_projection_runtime.lock().await;
    runtime.state = "dirty_last_good".to_string();
    runtime.last_defer_reason = Some(defer_reason.to_string());
    runtime.next_repair_at = Some(long_term_projection_repair_deadline(
        runtime.next_repair_at,
        Instant::now(),
    ));
}

fn long_term_projection_repair_deadline(
    existing_deadline: Option<Instant>,
    now: Instant,
) -> Instant {
    // Repeated terminal flushes and pressure deferrals may arrive while a
    // targeted repair is pending. Keep the first deadline so they cannot
    // postpone recovery indefinitely.
    existing_deadline.unwrap_or(now + LONG_TERM_PROJECTION_REPAIR_INTERVAL)
}

fn long_term_projection_pressure_retry_at(
    gate: &crate::db_pressure::DbPressureGate,
) -> Option<Instant> {
    match gate.background_deny_reason() {
        Some(crate::db_pressure::DbPressureDenyReason::PressureCooldown { remaining_ms }) => {
            Some(Instant::now() + Duration::from_millis(remaining_ms.max(1)))
        }
        Some(crate::db_pressure::DbPressureDenyReason::BackgroundBusy) | None => None,
    }
}

async fn wait_for_long_term_projection_pressure_retry(
    gate: &crate::db_pressure::DbPressureGate,
    observed_generation: u64,
    retry_at: Option<Instant>,
) {
    if let Some(retry_at) = retry_at {
        sleep(retry_at.saturating_duration_since(Instant::now())).await;
    } else {
        gate.wait_for_eligibility_change(observed_generation).await;
    }
}

fn long_term_projection_allows_expensive_repair(trigger: &str) -> bool {
    matches!(trigger, "repair_deadline" | "daily_verify")
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
    runtime.next_repair_at = Some(long_term_projection_repair_deadline(
        runtime.next_repair_at,
        Instant::now(),
    ));
}

async fn queue_long_term_projection_daily_verify(state: &AppState) -> Result<String> {
    let today = Utc::now().with_timezone(&Shanghai).date_naive().to_string();
    let control = LongTermProjectionWriteControl::background(
        &state.shutdown,
        crate::db_pressure::global_db_pressure_gate(),
    );
    queue_long_term_projection_daily_verify_with_control(&state.pool, &today, &control).await
}

async fn queue_long_term_projection_daily_verify_with_control(
    pool: &Pool<Sqlite>,
    daily_verify_date: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<String> {
    let pending = sqlx::query_as::<_, (i64, Option<String>)>(
        "SELECT COALESCE(daily_verify_pending, 0), daily_verify_bucket_date FROM long_term_projection_state WHERE consumer = ?1",
    )
    .bind(LONG_TERM_PROJECTION_CONSUMER)
    .fetch_optional(pool)
    .await?
    .unwrap_or((0, None));
    if pending.0 != 0 {
        if let Some(pending_date) = pending.1 {
            return Ok(pending_date);
        }

        // Databases created by the preceding schema revision can retain a pending bit without
        // its bucket date. Recover the existing durable repair before using a new calendar date.
        let recovered_date = sqlx::query_scalar::<_, String>(
            "SELECT bucket_date FROM long_term_projection_dirty_buckets ORDER BY CASE repair_reason WHEN 'daily_verify' THEN 0 ELSE 1 END, queued_at ASC, bucket_date ASC LIMIT 1",
        )
        .fetch_optional(pool)
        .await?
        .unwrap_or_else(|| daily_verify_date.to_string());
        let (mut tx, permit) = control.begin(pool).await?;
        sqlx::query(
            "UPDATE long_term_projection_state SET daily_verify_bucket_date = ?2, updated_at = datetime('now') WHERE consumer = ?1 AND daily_verify_pending = 1 AND daily_verify_bucket_date IS NULL",
        )
        .bind(LONG_TERM_PROJECTION_CONSUMER)
        .bind(&recovered_date)
        .execute(&mut *tx)
        .await?;
        control.commit(tx, permit).await?;
        return Ok(recovered_date);
    }

    let (mut tx, permit) = control.begin(pool).await?;
    let claimed = sqlx::query(
        "UPDATE long_term_projection_state SET daily_verify_pending = 1, daily_verify_bucket_date = ?2, updated_at = datetime('now') WHERE consumer = ?1 AND daily_verify_pending = 0",
    )
    .bind(LONG_TERM_PROJECTION_CONSUMER)
    .bind(daily_verify_date)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if claimed != 0 {
        sqlx::query(
            "INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason) VALUES (?1, 'daily_verify') ON CONFLICT(bucket_date) DO UPDATE SET repair_reason = excluded.repair_reason, generation = long_term_projection_dirty_buckets.generation + 1, next_attempt_at = NULL, updated_at = datetime('now')",
        )
        .bind(daily_verify_date)
        .execute(&mut *tx)
        .await?;
    }
    control.commit(tx, permit).await?;
    if claimed != 0 {
        Ok(daily_verify_date.to_string())
    } else {
        sqlx::query_scalar::<_, String>(
            "SELECT daily_verify_bucket_date FROM long_term_projection_state WHERE consumer = ?1 AND daily_verify_pending = 1",
        )
        .bind(LONG_TERM_PROJECTION_CONSUMER)
        .fetch_one(pool)
        .await
        .context("daily verification pending bucket disappeared while it was claimed")
    }
}

async fn long_term_projection_daily_verify_due(pool: &Pool<Sqlite>) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM long_term_projection_state WHERE consumer = ?1 AND (daily_verify_pending = 1 OR last_daily_verify_at IS NULL OR date(last_daily_verify_at, '+8 hours') < date('now', '+8 hours')))",
    )
    .bind(LONG_TERM_PROJECTION_CONSUMER)
    .fetch_one(pool)
    .await?
        != 0)
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
    let control = LongTermProjectionWriteControl::unrestricted();
    queue_long_term_projection_repairs_with_control(pool, dates, repair_reason, &control).await
}

async fn queue_long_term_projection_repairs_with_control(
    pool: &Pool<Sqlite>,
    dates: &[String],
    repair_reason: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    if dates.is_empty() {
        return Ok(());
    }
    for batch in dates.chunks(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS) {
        let (mut tx, permit) = control.begin(pool).await?;
        for date in batch {
            sqlx::query(
                "INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason) VALUES (?1, ?2) ON CONFLICT(bucket_date) DO UPDATE SET repair_reason = excluded.repair_reason, generation = long_term_projection_dirty_buckets.generation + 1, next_attempt_at = NULL, updated_at = datetime('now')",
            )
            .bind(date)
            .bind(repair_reason)
            .execute(&mut *tx)
            .await?;
        }
        control.commit(tx, permit).await?;
    }
    Ok(())
}

async fn ensure_long_term_projection_repairs(
    pool: &Pool<Sqlite>,
    dates: &[String],
    repair_reason: &str,
) -> Result<()> {
    let control = LongTermProjectionWriteControl::unrestricted();
    ensure_long_term_projection_repairs_with_control(pool, dates, repair_reason, &control).await
}

async fn ensure_long_term_projection_repairs_with_control(
    pool: &Pool<Sqlite>,
    dates: &[String],
    repair_reason: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    if dates.is_empty() {
        return Ok(());
    }
    for batch in dates.chunks(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS) {
        let (mut tx, permit) = control.begin(pool).await?;
        for date in batch {
            sqlx::query(
                "INSERT OR IGNORE INTO long_term_projection_dirty_buckets (bucket_date, repair_reason) VALUES (?1, ?2)",
            )
            .bind(date)
            .bind(repair_reason)
            .execute(&mut *tx)
            .await?;
        }
        control.commit(tx, permit).await?;
    }
    Ok(())
}

async fn load_long_term_projection_dirty_buckets(
    pool: &Pool<Sqlite>,
    dates: &[String],
) -> Result<Vec<LongTermProjectionDirtyBucket>> {
    if dates.is_empty() {
        return Ok(Vec::new());
    }
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT bucket_date, generation FROM long_term_projection_dirty_buckets WHERE bucket_date IN (",
    );
    let mut separated = builder.separated(", ");
    for date in dates {
        separated.push_bind(date);
    }
    separated.push_unseparated(")");
    Ok(builder
        .build_query_as::<LongTermProjectionDirtyBucket>()
        .fetch_all(pool)
        .await?)
}

async fn long_term_projection_repairs_are_deferred(
    pool: &Pool<Sqlite>,
    dates: &[String],
) -> Result<bool> {
    if dates.is_empty() {
        return Ok(false);
    }
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT EXISTS(SELECT 1 FROM long_term_projection_dirty_buckets WHERE datetime(next_attempt_at) > datetime('now') AND bucket_date IN (",
    );
    let mut separated = builder.separated(", ");
    for date in dates {
        separated.push_bind(date);
    }
    separated.push_unseparated("))");
    Ok(builder.build_query_scalar::<i64>().fetch_one(pool).await? != 0)
}

async fn defer_long_term_projection_repair(pool: &Pool<Sqlite>, bucket_date: &str) -> Result<()> {
    let control = LongTermProjectionWriteControl::unrestricted();
    defer_long_term_projection_repair_with_control(pool, bucket_date, &control).await
}

async fn defer_long_term_projection_repair_with_control(
    pool: &Pool<Sqlite>,
    bucket_date: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let (mut tx, permit) = control.begin(pool).await?;
    sqlx::query(
        "UPDATE long_term_projection_dirty_buckets SET next_attempt_at = datetime('now', '+5 minutes'), updated_at = datetime('now') WHERE bucket_date = ?1",
    )
    .bind(bucket_date)
    .execute(&mut *tx)
    .await?;
    control.commit(tx, permit).await?;
    Ok(())
}

async fn defer_long_term_projection_repairs(
    pool: &Pool<Sqlite>,
    bucket_dates: &[String],
) -> Result<()> {
    let control = LongTermProjectionWriteControl::unrestricted();
    defer_long_term_projection_repairs_with_control(pool, bucket_dates, &control).await
}

async fn defer_long_term_projection_repairs_with_control(
    pool: &Pool<Sqlite>,
    bucket_dates: &[String],
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    for bucket_date in bucket_dates {
        defer_long_term_projection_repair_with_control(pool, bucket_date, control).await?;
    }
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
        "SELECT state.bucket_date FROM long_term_projection_bucket_state state WHERE state.interval_baseline_ready = 1 AND NOT EXISTS (SELECT 1 FROM long_term_projection_dirty_buckets dirty LEFT JOIN long_term_projection_date_publications publication ON publication.publication_token = state.publication_token WHERE dirty.bucket_date = state.bucket_date AND (publication.published IS NULL OR publication.published = 0 OR state.publication_generation IS NULL OR dirty.generation <> state.publication_generation)) AND state.bucket_date IN (",
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
    let segments = collect_long_term_projection_interval_segments(&hourly, &daily, row.id);
    let mut bucket_dates = daily
        .keys()
        .map(|(bucket_date, _, _)| bucket_date.clone())
        .collect::<HashSet<_>>();
    for segment in &segments {
        bucket_dates.extend(long_term_projection_interval_dates(segment));
    }
    LongTermProjectionEvent {
        row_id: row.id,
        hourly,
        daily,
        segments,
        bucket_dates,
    }
}

fn long_term_projection_canonical_query(select: &str) -> String {
    format!(
        "{select} WHERE LOWER(TRIM(COALESCE(inv.status, ''))) NOT IN ('running', 'pending') AND instr(inv.occurred_at, 'T') = 0 AND inv.occurred_at >= ?1 AND inv.occurred_at < ?2"
    )
}

fn long_term_projection_crossing_text_query(select: &str) -> String {
    format!(
        "{select} WHERE LOWER(TRIM(COALESCE(inv.status, ''))) NOT IN ('running', 'pending') AND inv.occurred_at < ?1 AND CASE WHEN instr(inv.occurred_at, 'T') = 0 AND inv.t_total_ms IS NOT NULL AND inv.t_total_ms > 0 THEN julianday(inv.occurred_at) + inv.t_total_ms / 86400000.0 END >= julianday(?1)"
    )
}

async fn load_long_term_projection_live_rfc3339_compatibility(
    pool: &Pool<Sqlite>,
) -> Result<Option<LongTermRfc3339Compatibility>> {
    let terminal_filter = "instr(occurred_at, 'T') > 0 AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')";
    let has_rfc3339 = sqlx::query_scalar::<_, i64>(&format!(
        "SELECT 1 FROM codex_invocations WHERE {terminal_filter} LIMIT 1"
    ))
    .fetch_optional(pool)
    .await?;
    if has_rfc3339.is_none() {
        return Ok(None);
    }
    let max_duration_ms = sqlx::query_scalar::<_, f64>(&format!(
        "SELECT t_total_ms FROM codex_invocations WHERE {terminal_filter} AND t_total_ms IS NOT NULL AND t_total_ms > 0 ORDER BY t_total_ms DESC LIMIT 1"
    ))
    .fetch_optional(pool)
    .await?;
    Ok(Some(LongTermRfc3339Compatibility { max_duration_ms }))
}

fn long_term_projection_live_rfc3339_query(select: &str) -> String {
    let rfc3339_epoch = long_term_rfc3339_whole_epoch_seconds_sql("inv.occurred_at");
    let rfc3339_reaches_range_start =
        long_term_rfc3339_reaches_epoch_sql("inv.occurred_at", "inv.t_total_ms", "?3");
    format!(
        "{select} WHERE LOWER(TRIM(COALESCE(inv.status, ''))) NOT IN ('running', 'pending') AND instr(inv.occurred_at, 'T') > 0 AND inv.occurred_at >= ?1 AND inv.occurred_at < ?2 AND {rfc3339_epoch} < ?4 AND ({rfc3339_epoch} >= ?3 OR (inv.t_total_ms IS NOT NULL AND inv.t_total_ms > 0 AND {rfc3339_reaches_range_start}))"
    )
}

async fn invalidate_long_term_projection_interval_cache(state: &AppState) {
    let mut runtime = state.long_term_projection_runtime.lock().await;
    runtime.interval_index.clear();
    runtime.loaded_interval_dates.clear();
}

async fn flush_long_term_projection(
    state: &AppState,
    trigger: &'static str,
) -> Result<LongTermProjectionFlushOutcome> {
    let memory_baseline = state.memory_diagnostics.begin_operation(state).await;
    let result = run_long_term_projection_flush_with_retry(&state.shutdown, || {
        flush_long_term_projection_inner(state, trigger)
    })
    .await;
    let load_row_count = result.as_ref().copied().unwrap_or_default();
    state
        .memory_diagnostics
        .observe_operation(
            state,
            "long_term_projection_flush",
            memory_baseline,
            load_row_count,
            true,
        )
        .await;
    match result {
        Ok(_) => Ok(LongTermProjectionFlushOutcome::Completed),
        Err(error) if long_term_projection_write_is_pressure_deferred(&error) => {
            let mut runtime = state.long_term_projection_runtime.lock().await;
            runtime.state = "deferred".to_string();
            runtime.last_defer_reason = Some("writer_pressure".to_string());
            debug!(projection = "long_term", trigger, gate_outcome = "deferred", defer_reason = "writer_pressure", error = %error, "long-term projection flush deferred at a bounded write boundary");
            Ok(LongTermProjectionFlushOutcome::DeferredByPressure {
                retry_at: long_term_projection_pressure_retry_at(
                    crate::db_pressure::global_db_pressure_gate(),
                ),
            })
        }
        Err(error) if long_term_projection_write_is_deferred(&error) => Err(error),
        Err(error) => {
            let gate = crate::db_pressure::global_db_pressure_gate();
            if gate.record_error("long_term_projection_write", &error) {
                let mut runtime = state.long_term_projection_runtime.lock().await;
                runtime.state = "deferred".to_string();
                runtime.last_defer_reason = Some("writer_pressure".to_string());
                debug!(projection = "long_term", trigger, gate_outcome = "deferred", defer_reason = "sqlite_pressure", error = %error, "long-term projection flush deferred after a SQLite pressure error");
                Ok(LongTermProjectionFlushOutcome::DeferredByPressure {
                    retry_at: long_term_projection_pressure_retry_at(gate),
                })
            } else {
                Err(error)
            }
        }
    }
}

async fn run_long_term_projection_flush_with_retry<T, Operation, OperationFuture>(
    shutdown: &CancellationToken,
    operation: Operation,
) -> Result<T>
where
    Operation: FnMut() -> OperationFuture,
    OperationFuture: Future<Output = Result<T>>,
{
    run_long_term_projection_flush_with_retry_delays(
        shutdown,
        operation,
        &LONG_TERM_PROJECTION_LOCK_RETRY_DELAYS,
    )
    .await
}

async fn run_long_term_projection_flush_with_retry_delays<T, Operation, OperationFuture>(
    shutdown: &CancellationToken,
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
                    "long-term projection flush hit a SQLite lock; retrying"
                );
                tokio::select! {
                    _ = shutdown.cancelled() => bail!("long-term projection write cancelled during SQLite lock retry"),
                    _ = sleep(*delay) => {}
                }
            }
            Err(error) => return Err(error),
        }
    }
    operation().await
}

async fn flush_long_term_projection_inner(state: &AppState, trigger: &'static str) -> Result<u64> {
    // The initial refresher and P2 share date backups for crash recovery, but never their live
    // replacement window. Defer P2 while the refresher owns this process-wide maintenance lock;
    // a later terminal wake or ticker retries the bounded work.
    let _refresh_guard = match LONG_TERM_REFRESH_LOCK.try_lock() {
        Ok(guard) => guard,
        Err(_) => return Ok(0),
    };
    let control = LongTermProjectionWriteControl::background(
        &state.shutdown,
        crate::db_pressure::global_db_pressure_gate(),
    );

    let started = Instant::now();
    if trigger == "maintenance_deadline" {
        return advance_long_term_projection_maintenance(state, &control).await;
    }
    let mut cursor = load_long_term_projection_cursor_with_control(&state.pool, &control).await?;
    let daily_verify_requested =
        trigger == "daily_verify" || long_term_projection_daily_verify_due(&state.pool).await?;
    let daily_verify_date = if daily_verify_requested {
        Some(queue_long_term_projection_daily_verify(state).await?)
    } else {
        None
    };
    let state_row = load_long_term_state(&state.pool).await?;
    let mut baseline_cursor = None;
    let rollups_exist = long_term_rollups_exist(&state.pool).await?;
    if long_term_initial_materialization_needed(
        &state_row.status,
        state_row.last_error.as_deref(),
        rollups_exist,
    ) {
        // The dedicated refresher owns the first full materialization. Running it from this
        // P2 cursor worker bypasses pressure admission and can hold a competing writer lock.
        control.check()?;
        return Ok(0);
    } else if rollups_exist
        && (cursor == 0
            || matches!(
                state_row.status.as_str(),
                LONG_TERM_STATUS_RUNNING | LONG_TERM_STATUS_PREPARING | LONG_TERM_STATUS_DISABLED
            ))
    {
        // Existing rollups are a durable baseline after upgrade. Only the two open calendar
        // buckets need interval baselines before new terminal deltas can be merged exactly.
        baseline_cursor = Some(load_long_term_terminal_watermark(&state.pool).await?);
    }

    let mut repaired = Vec::new();
    let mut event_count = 0usize;
    let mut loaded_row_count = 0u64;
    let mut deferred_repair_count = 0usize;
    let mut deferred_repair_backoff_count = 0usize;
    if let Some(baseline_cursor) = baseline_cursor {
        let baseline_dates = long_term_projection_open_baseline_dates();
        queue_long_term_projection_repairs_with_control(
            &state.pool,
            &baseline_dates,
            "interval_baseline",
            &control,
        )
        .await?;
        let baseline_dirty =
            load_long_term_projection_dirty_buckets(&state.pool, &baseline_dates).await?;
        let mut rebuilds = Vec::with_capacity(baseline_dates.len());
        for date in &baseline_dates {
            let rebuild =
                build_long_term_projection_date_rebuild(&state.pool, date, &control).await?;
            loaded_row_count = loaded_row_count.saturating_add(rebuild.source_row_count);
            rebuilds.push(rebuild);
        }
        commit_long_term_projection_date_rebuilds_with_control(
            &state.pool,
            &rebuilds,
            Some(baseline_cursor),
            &baseline_dirty,
            state_row.status != LONG_TERM_STATUS_ERROR,
            &control,
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
        let mut batch_event_count = 0usize;
        let mut repair_event = None;
        for event in events {
            if !event.bucket_dates.is_subset(&ready_dates) {
                repair_event = Some(event);
                break;
            }
            let event_mutation_rows = event.hourly.len() + event.daily.len() + event.segments.len();
            if event_mutation_rows > LONG_TERM_PROJECTION_INCREMENTAL_MUTATION_ROWS {
                // A single event cannot fit beside its canonical interval, cursor, and state
                // updates. Rebuild all affected dates exactly instead of widening a write lock.
                repair_event = Some(event);
                break;
            }
            let additional_rollup_rows = event
                .hourly
                .keys()
                .filter(|key| !hourly.contains_key(*key))
                .count()
                + event
                    .daily
                    .keys()
                    .filter(|key| !daily.contains_key(*key))
                    .count();
            if batch_event_count > 0
                && hourly.len()
                    + daily.len()
                    + additional_rollup_rows
                    + segments.len()
                    + event.segments.len()
                    > LONG_TERM_PROJECTION_INCREMENTAL_MUTATION_ROWS
            {
                let outcome = apply_long_term_projection_incremental_with_runtime_and_control(
                    &state.pool,
                    &state.long_term_projection_runtime,
                    LongTermProjectionIncrementalBatch {
                        hourly: &hourly,
                        daily: &daily,
                        segments: &segments,
                    },
                    direct_cursor,
                    batch_event_count,
                    &control,
                )
                .await?;
                if outcome == LongTermProjectionIncrementalOutcome::RebuildRequired {
                    defer_long_term_projection_terminal_repair(state, "dirty_publication").await;
                    return Ok(0);
                }
                cursor = direct_cursor;
                hourly.clear();
                daily.clear();
                segments.clear();
                batch_event_count = 0;
            }
            direct_cursor = event.row_id;
            event_count = event_count.saturating_add(1);
            batch_event_count = batch_event_count.saturating_add(1);
            merge_long_term_projection_buckets(&mut hourly, event.hourly);
            merge_long_term_projection_buckets(&mut daily, event.daily);
            segments.extend(event.segments);
        }
        if batch_event_count > 0 {
            let outcome = apply_long_term_projection_incremental_with_runtime_and_control(
                &state.pool,
                &state.long_term_projection_runtime,
                LongTermProjectionIncrementalBatch {
                    hourly: &hourly,
                    daily: &daily,
                    segments: &segments,
                },
                direct_cursor,
                batch_event_count,
                &control,
            )
            .await?;
            if outcome == LongTermProjectionIncrementalOutcome::RebuildRequired {
                defer_long_term_projection_terminal_repair(state, "dirty_publication").await;
                return Ok(0);
            }
            cursor = direct_cursor;
        }
        if let Some(event) = repair_event {
            let mut repair_dates = event.bucket_dates.into_iter().collect::<Vec<_>>();
            repair_dates.sort();
            if !long_term_projection_allows_expensive_repair(trigger) {
                deferred_repair_count = deferred_repair_count.saturating_add(repair_dates.len());
                deferred_repair_backoff_count =
                    deferred_repair_backoff_count.saturating_add(repair_dates.len());
                debug!(
                    projection = "long_term",
                    repair_scope = ?repair_dates,
                    defer_reason = "terminal_hot_path",
                    "long-term cursor repair deferred to the bounded repair window"
                );
                defer_long_term_projection_terminal_repair(state, "terminal_hot_path").await;
            } else {
                ensure_long_term_projection_repairs_with_control(
                    &state.pool,
                    &repair_dates,
                    "interval_baseline",
                    &control,
                )
                .await?;
                let repair_dirty =
                    load_long_term_projection_dirty_buckets(&state.pool, &repair_dates).await?;
                if long_term_projection_repairs_are_deferred(&state.pool, &repair_dates).await? {
                    deferred_repair_count =
                        deferred_repair_count.saturating_add(repair_dates.len());
                    deferred_repair_backoff_count =
                        deferred_repair_backoff_count.saturating_add(repair_dates.len());
                    debug!(
                        projection = "long_term",
                        repair_scope = ?repair_dates,
                        defer_reason = "repair_backoff",
                        "long-term cursor repair retained until its retry deadline"
                    );
                } else {
                    let mut rebuilds = Vec::with_capacity(repair_dates.len());
                    let mut rebuild_error = None;
                    for date in &repair_dates {
                        match build_long_term_projection_date_rebuild(&state.pool, date, &control)
                            .await
                        {
                            Ok(rebuild) => {
                                loaded_row_count =
                                    loaded_row_count.saturating_add(rebuild.source_row_count);
                                rebuilds.push(rebuild);
                            }
                            Err(error) => {
                                rebuild_error = Some(error);
                                break;
                            }
                        }
                    }
                    if let Some(error) = rebuild_error {
                        defer_long_term_projection_repairs_with_control(
                            &state.pool,
                            &repair_dates,
                            &control,
                        )
                        .await?;
                        deferred_repair_count =
                            deferred_repair_count.saturating_add(repair_dates.len());
                        warn!(
                            error = %error,
                            projection = "long_term",
                            repair_scope = ?repair_dates,
                            retry_after_ms = 300_000_u64,
                            "long-term cursor repair deferred after an unavailable source"
                        );
                    } else {
                        commit_long_term_projection_date_rebuilds_with_control(
                            &state.pool,
                            &rebuilds,
                            Some(event.row_id),
                            &repair_dirty,
                            false,
                            &control,
                        )
                        .await?;
                        repaired.extend(repair_dates);
                        cursor = event.row_id;
                    }
                }
            }
        }
    }

    if !repaired.is_empty() {
        invalidate_long_term_projection_interval_cache(state).await;
    }

    let dirty_dates = if !long_term_projection_allows_expensive_repair(trigger) {
        Vec::new()
    } else {
        sqlx::query_as::<_, LongTermProjectionDirtyBucket>(
            "SELECT bucket_date, generation FROM long_term_projection_dirty_buckets WHERE next_attempt_at IS NULL OR datetime(next_attempt_at) <= datetime('now') ORDER BY queued_at ASC, bucket_date ASC LIMIT ?1",
        )
        .bind(LONG_TERM_PROJECTION_MAX_BUCKETS_PER_FLUSH)
        .fetch_all(&state.pool)
        .await?
    };
    for dirty in dirty_dates {
        let date = dirty.bucket_date.clone();
        if repaired.contains(&date) {
            continue;
        }
        let rebuild =
            match build_long_term_projection_date_rebuild(&state.pool, &date, &control).await {
                Ok(rebuild) => {
                    loaded_row_count = loaded_row_count.saturating_add(rebuild.source_row_count);
                    rebuild
                }
                Err(error) => {
                    defer_long_term_projection_repair_with_control(&state.pool, &date, &control)
                        .await?;
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
        commit_long_term_projection_date_rebuilds_with_control(
            &state.pool,
            &[rebuild],
            None,
            std::slice::from_ref(&dirty),
            false,
            &control,
        )
        .await?;
        repaired.push(date);
        invalidate_long_term_projection_interval_cache(state).await;
    }

    let (retention_pruned_hourly_rows, retention_pruned_interval_rows) = (0, 0);

    if let Some(daily_verify_date) = daily_verify_date {
        let maintenance_pending = long_term_projection_maintenance_needed(
            &state.pool,
            state.config.long_term_stats_hourly_retention_days,
        )
        .await?;
        complete_long_term_projection_daily_verify_with_control(
            &state.pool,
            &daily_verify_date,
            maintenance_pending,
            &control,
        )
        .await?;
    }

    let dirty_bucket_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_dirty_buckets")
            .fetch_one(&state.pool)
            .await?
            .max(0) as usize;
    deferred_repair_backoff_count = deferred_repair_backoff_count.max(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_projection_dirty_buckets WHERE datetime(next_attempt_at) > datetime('now')",
        )
        .fetch_one(&state.pool)
        .await?
        .max(0) as usize,
    );
    state
        .terminal_projection_hub
        .advance_long_term_cursor(cursor);
    let projection_health = state.terminal_projection_hub.health();
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let mut runtime = state.long_term_projection_runtime.lock().await;
    runtime.state = if deferred_repair_count > 0 {
        "dirty_last_good"
    } else if dirty_bucket_count == 0 {
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
    let terminal_hot_path_deferred =
        !long_term_projection_allows_expensive_repair(trigger) && deferred_repair_count > 0;
    runtime.last_defer_reason = if terminal_hot_path_deferred {
        Some("terminal_hot_path".to_string())
    } else if deferred_repair_backoff_count > 0 {
        Some("repair_backoff".to_string())
    } else if deferred_repair_count > 0 {
        Some("repair_source_unavailable".to_string())
    } else {
        None
    };
    runtime.last_error_kind = (!terminal_hot_path_deferred && deferred_repair_count > 0)
        .then(|| "targeted_repair".to_string());
    runtime.next_repair_at = if deferred_repair_count > 0 {
        Some(long_term_projection_repair_deadline(
            runtime.next_repair_at,
            Instant::now(),
        ))
    } else {
        None
    };
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
        deferred_repair_backoff_count,
        retention_pruned_hourly_rows,
        retention_pruned_interval_rows,
        flush_outcome = "accepted",
        elapsed_ms,
        "long-term projection flush completed"
    );
    drop(runtime);
    Ok(loaded_row_count.saturating_add(event_count as u64))
}

async fn advance_long_term_projection_maintenance(
    state: &AppState,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<u64> {
    // Maintenance is intentionally a single durable continuation step. Each helper makes at
    // most one 512-row transaction and re-checks cancellation and database pressure before it
    // starts. Letting a deadline exhaust every backlog would recreate the writer starvation
    // this worker is intended to avoid.
    if finish_long_term_projection_publication_cleanup(&state.pool, control).await? {
        return Ok(0);
    }
    if migrate_long_term_projection_legacy_interval_state(&state.pool, control).await? {
        return Ok(0);
    }
    let (hourly, intervals) = prune_long_term_projection_hourly_retention_with_control(
        &state.pool,
        state.config.long_term_stats_hourly_retention_days,
        control,
    )
    .await?;
    Ok(hourly.saturating_add(intervals))
}

async fn long_term_rollups_exist(pool: &Pool<Sqlite>) -> Result<bool> {
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM long_term_usage_daily LIMIT 1)")
            .fetch_one(pool)
            .await?
            != 0,
    )
}

fn long_term_initial_materialization_needed(
    status: &str,
    last_error: Option<&str>,
    rollups_exist: bool,
) -> bool {
    status == LONG_TERM_STATUS_ERROR
        || last_error == Some(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR)
        || (!matches!(status, LONG_TERM_STATUS_READY | LONG_TERM_STATUS_EMPTY) && !rollups_exist)
}

async fn complete_long_term_projection_daily_verify_with_control(
    pool: &Pool<Sqlite>,
    daily_verify_date: &str,
    maintenance_pending: bool,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    if maintenance_pending {
        return Ok(());
    }
    let today = Utc::now().with_timezone(&Shanghai).date_naive().to_string();
    let (mut transaction, permit) = control.begin(pool).await?;
    sqlx::query(
        "UPDATE long_term_projection_state SET daily_verify_pending = 0, daily_verify_bucket_date = NULL, last_daily_verify_at = CASE WHEN ?2 = ?3 THEN datetime('now') ELSE last_daily_verify_at END, updated_at = datetime('now') WHERE consumer = ?1 AND daily_verify_pending = 1 AND daily_verify_bucket_date = ?2 AND NOT EXISTS (SELECT 1 FROM long_term_projection_dirty_buckets WHERE bucket_date = ?2)",
    )
    .bind(LONG_TERM_PROJECTION_CONSUMER)
    .bind(daily_verify_date)
    .bind(&today)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await
}

fn long_term_projection_hourly_retention_start_date(retention_days: u64) -> NaiveDate {
    Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(retention_days.max(366) as i64 - 1)
}

async fn prune_long_term_projection_hourly_retention(
    pool: &Pool<Sqlite>,
    retention_days: u64,
) -> Result<(u64, u64)> {
    let control = LongTermProjectionWriteControl::unrestricted();
    prune_long_term_projection_hourly_retention_with_control(pool, retention_days, &control).await
}

#[derive(Debug, Clone, Copy)]
enum LongTermProjectionRetentionTarget {
    HourlyRollup,
    LegacyInterval,
    CanonicalInterval,
    Suppression,
    RebuildMember,
}

async fn long_term_projection_hourly_retention_target(
    pool: &Pool<Sqlite>,
    retention_start_date: NaiveDate,
    retention_start_epoch: i64,
) -> Result<Option<LongTermProjectionRetentionTarget>> {
    let retention_start_ms = retention_start_epoch * 1_000;
    let candidates = [
        (
            LongTermProjectionRetentionTarget::HourlyRollup,
            "SELECT EXISTS(SELECT 1 FROM long_term_usage_hourly WHERE bucket_start_epoch < ?1 LIMIT 1)",
            retention_start_epoch,
        ),
        (
            LongTermProjectionRetentionTarget::LegacyInterval,
            "SELECT EXISTS(SELECT 1 FROM long_term_projection_intervals WHERE bucket_kind = 'hourly' AND bucket_date < ?1 LIMIT 1)",
            0,
        ),
        (
            LongTermProjectionRetentionTarget::CanonicalInterval,
            "SELECT EXISTS(SELECT 1 FROM long_term_projection_interval_state WHERE interval_end_ms < ?1 LIMIT 1)",
            retention_start_ms,
        ),
    ];
    for (target, statement, value) in candidates {
        let mut query = sqlx::query_scalar::<_, i64>(statement);
        if matches!(target, LongTermProjectionRetentionTarget::LegacyInterval) {
            query = query.bind(retention_start_date.to_string());
        } else {
            query = query.bind(value);
        }
        if query.fetch_one(pool).await? != 0 {
            return Ok(Some(target));
        }
    }
    for (target, table) in [
        (
            LongTermProjectionRetentionTarget::Suppression,
            "long_term_projection_interval_suppressions",
        ),
        (
            LongTermProjectionRetentionTarget::RebuildMember,
            "long_term_projection_rebuild_members",
        ),
    ] {
        let statement = format!(
            "SELECT EXISTS(SELECT 1 FROM {table} metadata WHERE NOT EXISTS (SELECT 1 FROM long_term_projection_interval_state state WHERE state.invocation_row_id = metadata.invocation_row_id) LIMIT 1)"
        );
        if sqlx::query_scalar::<_, i64>(&statement)
            .fetch_one(pool)
            .await?
            != 0
        {
            return Ok(Some(target));
        }
    }
    Ok(None)
}

async fn prune_long_term_projection_hourly_retention_with_control(
    pool: &Pool<Sqlite>,
    retention_days: u64,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<(u64, u64)> {
    let retention_start_date = long_term_projection_hourly_retention_start_date(retention_days);
    let retention_start_epoch = retention_start_date
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Shanghai.from_local_datetime(&value).single())
        .map(|value| value.timestamp())
        .context("invalid long-term projection hourly retention start")?;
    let Some(target) = long_term_projection_hourly_retention_target(
        pool,
        retention_start_date,
        retention_start_epoch,
    )
    .await?
    else {
        return Ok((0, 0));
    };

    let (mut tx, permit) = control.begin(pool).await?;
    let deleted = match target {
        LongTermProjectionRetentionTarget::HourlyRollup => {
            sqlx::query(
                "DELETE FROM long_term_usage_hourly WHERE rowid IN (SELECT rowid FROM long_term_usage_hourly WHERE bucket_start_epoch < ?1 LIMIT ?2)",
            )
            .bind(retention_start_epoch)
            .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
            .execute(&mut *tx)
            .await?
            .rows_affected()
        }
        LongTermProjectionRetentionTarget::LegacyInterval => {
            sqlx::query(
                "DELETE FROM long_term_projection_intervals WHERE rowid IN (SELECT rowid FROM long_term_projection_intervals WHERE bucket_kind = 'hourly' AND bucket_date < ?1 LIMIT ?2)",
            )
            .bind(retention_start_date.to_string())
            .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
            .execute(&mut *tx)
            .await?
            .rows_affected()
        }
        LongTermProjectionRetentionTarget::CanonicalInterval => {
            sqlx::query(
                "DELETE FROM long_term_projection_interval_state WHERE rowid IN (SELECT rowid FROM long_term_projection_interval_state WHERE interval_end_ms < ?1 LIMIT ?2)",
            )
            .bind(retention_start_epoch * 1_000)
            .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
            .execute(&mut *tx)
            .await?
            .rows_affected()
        }
        LongTermProjectionRetentionTarget::Suppression => {
            sqlx::query(
                "DELETE FROM long_term_projection_interval_suppressions WHERE rowid IN (SELECT metadata.rowid FROM long_term_projection_interval_suppressions metadata WHERE NOT EXISTS (SELECT 1 FROM long_term_projection_interval_state state WHERE state.invocation_row_id = metadata.invocation_row_id) LIMIT ?1)",
            )
            .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
            .execute(&mut *tx)
            .await?
            .rows_affected()
        }
        LongTermProjectionRetentionTarget::RebuildMember => {
            sqlx::query(
                "DELETE FROM long_term_projection_rebuild_members WHERE rowid IN (SELECT metadata.rowid FROM long_term_projection_rebuild_members metadata WHERE NOT EXISTS (SELECT 1 FROM long_term_projection_interval_state state WHERE state.invocation_row_id = metadata.invocation_row_id) LIMIT ?1)",
            )
            .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
            .execute(&mut *tx)
            .await?
            .rows_affected()
        }
    };
    control.commit(tx, permit).await?;
    Ok(match target {
        LongTermProjectionRetentionTarget::HourlyRollup => (deleted, 0),
        LongTermProjectionRetentionTarget::LegacyInterval
        | LongTermProjectionRetentionTarget::CanonicalInterval
        | LongTermProjectionRetentionTarget::Suppression
        | LongTermProjectionRetentionTarget::RebuildMember => (0, deleted),
    })
}

async fn load_long_term_terminal_watermark(pool: &Pool<Sqlite>) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(id), 0) FROM codex_invocations WHERE LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')",
    )
    .fetch_one(pool)
    .await?)
}

async fn load_long_term_projection_cursor(pool: &Pool<Sqlite>) -> Result<i64> {
    let control = LongTermProjectionWriteControl::unrestricted();
    load_long_term_projection_cursor_with_control(pool, &control).await
}

async fn load_long_term_projection_cursor_with_control(
    pool: &Pool<Sqlite>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<i64> {
    let (mut tx, permit) = control.begin(pool).await?;
    sqlx::query(
        "INSERT OR IGNORE INTO long_term_projection_state (consumer, cursor_row_id) VALUES (?1, 0)",
    )
    .bind(LONG_TERM_PROJECTION_CONSUMER)
    .execute(&mut *tx)
    .await?;
    control.commit(tx, permit).await?;
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
    source_row_count: u64,
}

#[derive(Debug)]
struct LongTermProjectionRebuildPublication<'a> {
    next_cursor: Option<i64>,
    clear_dirty_buckets: &'a [LongTermProjectionDirtyBucket],
    mark_ready: bool,
    publish_state: bool,
    publication_token: Option<&'a str>,
    repaired_start_date: Option<&'a str>,
}

#[derive(Debug, Clone, FromRow)]
struct LongTermProjectionDirtyBucket {
    bucket_date: String,
    generation: i64,
}

#[derive(Debug, FromRow)]
struct LongTermProjectionPublicationMember {
    bucket_date: String,
    rebuild_token: String,
    publication_token: String,
    publication_generation: Option<i64>,
}

fn next_long_term_projection_publication_token() -> String {
    let sequence =
        LONG_TERM_PROJECTION_PUBLICATION_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    format!(
        "long-term-publication:{}:{sequence}",
        Utc::now().timestamp_micros()
    )
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
    control: &LongTermProjectionWriteControl<'_>,
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
    let select = format!(
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
        "#,
    );
    let start_text = start.format("%Y-%m-%d %H:%M:%S").to_string();
    let end_text = end.format("%Y-%m-%d %H:%M:%S").to_string();
    let canonical_query = long_term_projection_canonical_query(&select);
    let mut rows = sqlx::query_as::<_, LongTermInvocationRow>(&canonical_query)
        .bind(&start_text)
        .bind(&end_text)
        .fetch_all(pool)
        .await?;
    let crossing_text_query = long_term_projection_crossing_text_query(&select);
    rows.extend(
        sqlx::query_as::<_, LongTermInvocationRow>(&crossing_text_query)
            .bind(&start_text)
            .fetch_all(pool)
            .await?,
    );
    if let Some(rfc3339_compatibility) =
        load_long_term_projection_live_rfc3339_compatibility(pool).await?
    {
        let (rfc3339_lower, rfc3339_upper) =
            long_term_rfc3339_text_bounds(start, end, &rfc3339_compatibility);
        let rfc3339_query = long_term_projection_live_rfc3339_query(&select);
        rows.extend(
            sqlx::query_as::<_, LongTermInvocationRow>(&rfc3339_query)
                .bind(rfc3339_lower)
                .bind(rfc3339_upper)
                .bind(start.timestamp())
                .bind(end.timestamp())
                .fetch_all(pool)
                .await?,
        );
    }
    rows.sort_by(|left, right| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then_with(|| left.id.cmp(&right.id))
    });
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
        let archive_sha256 = load_long_term_archive_sha256(pool, archive_path.file_path())
            .await?
            .context("completed invocation archive has no manifest sha256")?;
        ensure_long_term_archive_source_identity(
            pool,
            "codex_invocations",
            archive_path.file_path(),
            &archive_sha256,
        )
        .await?;
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
        let archive_fingerprint = match long_term_archive_pool_fingerprint(&archive_pool).await {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                archive_pool.close().await;
                drop(cleanup);
                return Err(error);
            }
        };
        let archive_rows = async {
            let archive_query = long_term_archive_invocation_query_for_range(&archive_pool).await?;
            let compatibility = load_or_inspect_long_term_archive_compatibility(
                pool,
                &archive_pool,
                archive_path.file_path(),
                &archive_fingerprint,
                &archive_query.parts,
                control,
            )
            .await?;
            load_long_term_archive_invocation_rows_for_range(
                &archive_pool,
                &archive_query,
                compatibility,
                start,
                end,
            )
            .await
        }
        .await;
        archive_pool.close().await;
        drop(cleanup);
        let archive_rows = archive_rows?;
        ensure_long_term_archive_source_identity(
            pool,
            "codex_invocations",
            archive_path.file_path(),
            &archive_sha256,
        )
        .await?;
        for mut row in archive_rows {
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
    control: &LongTermProjectionWriteControl<'_>,
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
    let mut rows = load_long_term_projection_rows_for_date(pool, date, start, end, control).await?;
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
    interval_segments
        .retain(|segment| long_term_projection_interval_dates(segment).contains(bucket_date));
    Ok(LongTermProjectionDateRebuild {
        bucket_date: bucket_date.to_string(),
        start_epoch: start.timestamp(),
        end_epoch: end.timestamp(),
        hourly,
        daily,
        interval_segments,
        source_row_count: rows.len() as u64,
    })
}

async fn commit_long_term_projection_date_rebuilds(
    pool: &Pool<Sqlite>,
    rebuilds: &[LongTermProjectionDateRebuild],
    next_cursor: Option<i64>,
    clear_dirty_buckets: &[LongTermProjectionDirtyBucket],
    mark_ready: bool,
) -> Result<()> {
    let control = LongTermProjectionWriteControl::unrestricted();
    commit_long_term_projection_date_rebuilds_with_control(
        pool,
        rebuilds,
        next_cursor,
        clear_dirty_buckets,
        mark_ready,
        &control,
    )
    .await
}

async fn clear_long_term_projection_rebuild_members(
    pool: &Pool<Sqlite>,
    rebuild_token: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    loop {
        let (mut transaction, permit) = control.begin(pool).await?;
        let deleted = sqlx::query(
            "DELETE FROM long_term_projection_rebuild_members WHERE rowid IN (SELECT rowid FROM long_term_projection_rebuild_members WHERE rebuild_token = ?1 LIMIT ?2)",
        )
        .bind(rebuild_token)
        .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        control.commit(transaction, permit).await?;
        if deleted == 0 {
            return Ok(());
        }
    }
}

async fn clear_long_term_projection_daily_backup(
    pool: &Pool<Sqlite>,
    rebuild_token: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    loop {
        let has_backup = sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM long_term_projection_daily_backups WHERE rebuild_token = ?1)",
        )
        .bind(rebuild_token)
        .fetch_one(pool)
        .await?
            != 0;
        if !has_backup {
            return Ok(());
        }
        let (mut transaction, permit) = control.begin(pool).await?;
        let deleted = sqlx::query(
            "DELETE FROM long_term_projection_daily_backups WHERE rowid IN (SELECT rowid FROM long_term_projection_daily_backups WHERE rebuild_token = ?1 LIMIT ?2)",
        )
        .bind(rebuild_token)
        .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        control.commit(transaction, permit).await?;
        if deleted == 0 {
            return Ok(());
        }
    }
}

async fn finish_long_term_projection_backup_cleanup(
    pool: &Pool<Sqlite>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<bool> {
    let (mut transaction, permit) = control.begin(pool).await?;
    let deleted = sqlx::query(
        r#"
            DELETE FROM long_term_projection_daily_backups
            WHERE rowid IN (
                SELECT backup.rowid
                FROM long_term_projection_daily_backups backup
                JOIN long_term_projection_bucket_state state
                  ON state.bucket_date = backup.stats_date
                 AND state.active_daily_backup_token IS NULL
                 AND state.publication_token = 'cleanup:' || backup.rebuild_token
                WHERE NOT EXISTS (
                    SELECT 1
                    FROM long_term_projection_daily_backup_claims claim
                    WHERE claim.bucket_date = state.bucket_date
                      AND claim.rebuild_token = backup.rebuild_token
                )
                ORDER BY backup.rowid
                LIMIT ?1
            )
            "#,
    )
    .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    if deleted != 0 {
        control.commit(transaction, permit).await?;
        return Ok(true);
    }

    let cleared = sqlx::query(
        r#"
            UPDATE long_term_projection_bucket_state
            SET publication_token = NULL, updated_at = datetime('now')
            WHERE rowid IN (
                SELECT state.rowid
                FROM long_term_projection_bucket_state state
                WHERE state.active_daily_backup_token IS NULL
                  AND state.publication_token LIKE 'cleanup:%'
                  AND NOT EXISTS (
                    SELECT 1
                    FROM long_term_projection_daily_backups backup
                    WHERE backup.rebuild_token = substr(state.publication_token, 9)
                  )
                ORDER BY state.rowid
                LIMIT ?1
            )
            "#,
    )
    .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    control.commit(transaction, permit).await?;
    Ok(cleared != 0)
}

async fn release_long_term_projection_daily_backups(
    pool: &Pool<Sqlite>,
    backups: &[(String, String)],
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    for batch in backups.chunks(LONG_TERM_PROJECTION_REBUILD_PUBLICATION_DATES) {
        let (mut transaction, permit) = control.begin(pool).await?;
        let mut query = QueryBuilder::<Sqlite>::new(
            "UPDATE long_term_projection_bucket_state SET active_daily_backup_token = NULL, publication_token = 'cleanup:' || active_daily_backup_token, publication_generation = NULL, updated_at = datetime('now') WHERE ",
        );
        for (index, (bucket_date, rebuild_token)) in batch.iter().enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("(bucket_date = ")
                .push_bind(bucket_date)
                .push(" AND active_daily_backup_token = ")
                .push_bind(rebuild_token)
                .push(")");
        }
        query.build().execute(&mut *transaction).await?;
        let mut query = QueryBuilder::<Sqlite>::new(
            "DELETE FROM long_term_projection_daily_backup_claims WHERE ",
        );
        for (index, (bucket_date, rebuild_token)) in batch.iter().enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("(bucket_date = ")
                .push_bind(bucket_date)
                .push(" AND rebuild_token = ")
                .push_bind(rebuild_token)
                .push(")");
        }
        query.build().execute(&mut *transaction).await?;
        control.commit(transaction, permit).await?;
    }
    let _ = finish_long_term_projection_backup_cleanup(pool, control).await?;
    Ok(())
}

async fn ensure_long_term_projection_daily_backup(
    pool: &Pool<Sqlite>,
    rebuild: &LongTermProjectionDateRebuild,
    rebuild_token: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    ensure_long_term_projection_daily_backup_for_date(
        pool,
        &rebuild.bucket_date,
        rebuild_token,
        true,
        control,
    )
    .await
}

async fn stage_long_term_projection_date_publication(
    pool: &Pool<Sqlite>,
    bucket_date: &str,
    rebuild_token: &str,
    publication_token: &str,
    publication_generation: Option<i64>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let (mut transaction, permit) = control.begin(pool).await?;
    let staged = sqlx::query(
        "UPDATE long_term_projection_bucket_state SET publication_token = ?1, publication_generation = ?2, updated_at = datetime('now') WHERE bucket_date = ?3 AND active_daily_backup_token = ?4",
    )
    .bind(publication_token)
    .bind(publication_generation)
    .bind(bucket_date)
    .bind(rebuild_token)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    if staged != 1 {
        bail!("long-term projection date publication lost its backup for {bucket_date}");
    }
    control.commit(transaction, permit).await
}

async fn ensure_long_term_projection_daily_backup_for_date(
    pool: &Pool<Sqlite>,
    bucket_date: &str,
    rebuild_token: &str,
    reset_interval_baseline: bool,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let (mut transaction, permit) = control.begin(pool).await?;
    let active = sqlx::query_scalar::<_, Option<String>>(
        "SELECT active_daily_backup_token FROM long_term_projection_bucket_state WHERE bucket_date = ?1",
    )
    .bind(bucket_date)
    .fetch_optional(&mut *transaction)
    .await?;
    let active = active.flatten();
    if let Some(active) = active.as_deref()
        && active != rebuild_token
    {
        bail!("long-term projection daily backup for {bucket_date} is owned by {active}");
    }
    let owner = sqlx::query_scalar::<_, String>(
        "SELECT rebuild_token FROM long_term_projection_daily_backup_claims WHERE bucket_date = ?1",
    )
    .bind(bucket_date)
    .fetch_optional(&mut *transaction)
    .await?;
    if let Some(owner) = owner.as_deref()
        && owner != rebuild_token
    {
        bail!("long-term projection daily backup for {bucket_date} is owned by {owner}");
    }
    if owner.is_none() {
        sqlx::query(
            "INSERT INTO long_term_projection_daily_backup_claims (bucket_date, rebuild_token) VALUES (?1, ?2)",
        )
        .bind(bucket_date)
        .bind(rebuild_token)
        .execute(&mut *transaction)
        .await?;
    }
    control.commit(transaction, permit).await?;

    // A previous worker may have completed the backup publication before cancellation. Keep
    // that complete snapshot live while the same owner resumes the replacement.
    if active.as_deref() == Some(rebuild_token) {
        return Ok(());
    }

    clear_long_term_projection_daily_backup(pool, rebuild_token, control).await?;
    let daily_row_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM long_term_usage_daily WHERE stats_date = ?1",
    )
    .bind(bucket_date)
    .fetch_one(pool)
    .await?;
    let mut offset = 0_i64;
    while offset < daily_row_count {
        let (mut transaction, permit) = control.begin(pool).await?;
        let copied = sqlx::query(
            r#"
            INSERT INTO long_term_projection_daily_backups (
                rebuild_token, stats_date, dimension, series_key, display_name, reasoning_effort,
                calls, token_total, token_samples, cost_total, cost_samples, usage_time_ms,
                usage_time_samples, wall_time_ms, wall_time_samples, output_tokens_total,
                stream_duration_ms, output_speed_samples, first_byte_sum_ms,
                first_byte_samples, response_sum_ms, response_samples
            )
            SELECT ?1, stats_date, dimension, series_key, display_name, reasoning_effort,
                calls, token_total, token_samples, cost_total, cost_samples, usage_time_ms,
                usage_time_samples, wall_time_ms, wall_time_samples, output_tokens_total,
                stream_duration_ms, output_speed_samples, first_byte_sum_ms,
                first_byte_samples, response_sum_ms, response_samples
            FROM long_term_usage_daily
            WHERE stats_date = ?2
            ORDER BY dimension, series_key
            LIMIT ?3 OFFSET ?4
            "#,
        )
        .bind(rebuild_token)
        .bind(bucket_date)
        .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
        .bind(offset)
        .execute(&mut *transaction)
        .await?
        .rows_affected() as i64;
        control.commit(transaction, permit).await?;
        offset += copied;
    }

    let (mut transaction, permit) = control.begin(pool).await?;
    if reset_interval_baseline {
        sqlx::query(
            "INSERT INTO long_term_projection_bucket_state (bucket_date, interval_baseline_ready, active_daily_backup_token) VALUES (?1, 0, ?2) ON CONFLICT(bucket_date) DO UPDATE SET interval_baseline_ready = 0, active_daily_backup_token = excluded.active_daily_backup_token, updated_at = datetime('now')",
        )
        .bind(bucket_date)
        .bind(rebuild_token)
        .execute(&mut *transaction)
        .await?;
    } else {
        sqlx::query(
            "INSERT INTO long_term_projection_bucket_state (bucket_date, active_daily_backup_token) VALUES (?1, ?2) ON CONFLICT(bucket_date) DO UPDATE SET active_daily_backup_token = excluded.active_daily_backup_token, updated_at = datetime('now')",
        )
        .bind(bucket_date)
        .bind(rebuild_token)
        .execute(&mut *transaction)
        .await?;
    }
    control.commit(transaction, permit).await
}

async fn replace_long_term_projection_date_rollups(
    pool: &Pool<Sqlite>,
    rebuild: &LongTermProjectionDateRebuild,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let hourly = rebuild.hourly.values().collect::<Vec<_>>();
    for batch in hourly.chunks(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS) {
        let (mut transaction, permit) = control.begin(pool).await?;
        for bucket in batch {
            insert_long_term_hourly(&mut transaction, bucket).await?;
        }
        control.commit(transaction, permit).await?;
    }
    let daily = rebuild.daily.values().collect::<Vec<_>>();
    for batch in daily.chunks(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS) {
        let (mut transaction, permit) = control.begin(pool).await?;
        for bucket in batch {
            insert_long_term_daily(&mut transaction, bucket).await?;
        }
        control.commit(transaction, permit).await?;
    }

    // Upsert fresh rows before removing obsolete keys so a read cannot observe an empty date
    // between bounded transactions. The dirty marker remains until this entire replacement ends.
    let existing_daily = sqlx::query_as::<_, (String, String)>(
        "SELECT dimension, series_key FROM long_term_usage_daily WHERE stats_date = ?1",
    )
    .bind(&rebuild.bucket_date)
    .fetch_all(pool)
    .await?;
    let daily_keys = daily
        .iter()
        .map(|bucket| (bucket.dimension.as_str(), bucket.series_key.as_str()))
        .collect::<HashSet<_>>();
    let obsolete_daily = existing_daily
        .into_iter()
        .filter(|(dimension, series_key)| {
            !daily_keys.contains(&(dimension.as_str(), series_key.as_str()))
        })
        .collect::<Vec<_>>();
    for batch in obsolete_daily.chunks(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS) {
        let (mut transaction, permit) = control.begin(pool).await?;
        for (dimension, series_key) in batch {
            sqlx::query(
                "DELETE FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = ?2 AND series_key = ?3",
            )
            .bind(&rebuild.bucket_date)
            .bind(dimension)
            .bind(series_key)
            .execute(&mut *transaction)
            .await?;
        }
        control.commit(transaction, permit).await?;
    }

    let existing_hourly = sqlx::query_as::<_, (i64, String, String)>(
        "SELECT bucket_start_epoch, dimension, series_key FROM long_term_usage_hourly WHERE bucket_start_epoch >= ?1 AND bucket_start_epoch < ?2",
    )
    .bind(rebuild.start_epoch)
    .bind(rebuild.end_epoch)
    .fetch_all(pool)
    .await?;
    let hourly_keys = hourly
        .iter()
        .map(|bucket| {
            (
                bucket.bucket_start_epoch,
                bucket.dimension.as_str(),
                bucket.series_key.as_str(),
            )
        })
        .collect::<HashSet<_>>();
    let obsolete_hourly = existing_hourly
        .into_iter()
        .filter(|(bucket_start_epoch, dimension, series_key)| {
            !hourly_keys.contains(&(*bucket_start_epoch, dimension.as_str(), series_key.as_str()))
        })
        .collect::<Vec<_>>();
    for batch in obsolete_hourly.chunks(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS) {
        let (mut transaction, permit) = control.begin(pool).await?;
        for (bucket_start_epoch, dimension, series_key) in batch {
            sqlx::query(
                "DELETE FROM long_term_usage_hourly WHERE bucket_start_epoch = ?1 AND dimension = ?2 AND series_key = ?3",
            )
            .bind(bucket_start_epoch)
            .bind(dimension)
            .bind(series_key)
            .execute(&mut *transaction)
            .await?;
        }
        control.commit(transaction, permit).await?;
    }

    let (mut transaction, permit) = control.begin(pool).await?;
    sqlx::query(
        "INSERT INTO long_term_projection_bucket_state (bucket_date, interval_baseline_ready) VALUES (?1, 1) ON CONFLICT(bucket_date) DO UPDATE SET interval_baseline_ready = 1, updated_at = datetime('now')",
    )
    .bind(&rebuild.bucket_date)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await
}

async fn commit_long_term_projection_date_rebuilds_with_control(
    pool: &Pool<Sqlite>,
    rebuilds: &[LongTermProjectionDateRebuild],
    next_cursor: Option<i64>,
    clear_dirty_buckets: &[LongTermProjectionDirtyBucket],
    mark_ready: bool,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    if rebuilds.is_empty() {
        return commit_long_term_projection_date_rebuild_chunk_with_control(
            pool,
            rebuilds,
            LongTermProjectionRebuildPublication {
                next_cursor,
                clear_dirty_buckets,
                mark_ready,
                publish_state: true,
                publication_token: None,
                repaired_start_date: None,
            },
            control,
        )
        .await;
    }
    let publication_token = next_long_term_projection_publication_token();
    let repaired_start_date = rebuilds
        .iter()
        .filter(|rebuild| !rebuild.daily.is_empty())
        .map(|rebuild| rebuild.bucket_date.as_str())
        .min()
        .map(str::to_string);
    for (index, rebuild_chunk) in rebuilds
        .chunks(LONG_TERM_PROJECTION_REBUILD_PUBLICATION_DATES)
        .enumerate()
    {
        let last_chunk =
            (index + 1) * LONG_TERM_PROJECTION_REBUILD_PUBLICATION_DATES >= rebuilds.len();
        let chunk_dates = rebuild_chunk
            .iter()
            .map(|rebuild| rebuild.bucket_date.as_str())
            .collect::<HashSet<_>>();
        let chunk_dirty = clear_dirty_buckets
            .iter()
            .filter(|dirty| chunk_dates.contains(dirty.bucket_date.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        // Staged chunks retain their backup pointer and dirty generation. The final bounded
        // transaction publishes a token with the cursor; readers do not expose a staged prefix.
        commit_long_term_projection_date_rebuild_chunk_with_control(
            pool,
            rebuild_chunk,
            LongTermProjectionRebuildPublication {
                next_cursor: last_chunk.then_some(next_cursor).flatten(),
                clear_dirty_buckets: &chunk_dirty,
                mark_ready: last_chunk && mark_ready,
                publish_state: last_chunk,
                publication_token: Some(&publication_token),
                repaired_start_date: last_chunk
                    .then_some(repaired_start_date.as_deref())
                    .flatten(),
            },
            control,
        )
        .await?;
        if !last_chunk {
            control.complete_rebuild_chunk();
            control.check()?;
        }
    }
    release_long_term_projection_date_publication(
        pool,
        rebuilds,
        clear_dirty_buckets,
        &publication_token,
        control,
    )
    .await?;
    Ok(())
}

async fn commit_long_term_projection_date_rebuild_chunk_with_control(
    pool: &Pool<Sqlite>,
    rebuilds: &[LongTermProjectionDateRebuild],
    publication: LongTermProjectionRebuildPublication<'_>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    debug_assert!(rebuilds.len() <= LONG_TERM_PROJECTION_REBUILD_PUBLICATION_DATES);
    let chunk_repaired_start_date = rebuilds
        .iter()
        .filter(|rebuild| !rebuild.daily.is_empty())
        .map(|rebuild| rebuild.bucket_date.as_str())
        .min()
        .map(str::to_string);
    let repaired_start_date = publication
        .repaired_start_date
        .map(str::to_string)
        .or(chunk_repaired_start_date);
    let repaired_nonempty = repaired_start_date.is_some();

    let mut rebuild_tokens = Vec::with_capacity(rebuilds.len());
    for rebuild in rebuilds {
        let token = format!("long-term-date:{}", rebuild.bucket_date);
        ensure_long_term_projection_daily_backup(pool, rebuild, &token, control).await?;
        if let Some(publication_token) = publication.publication_token {
            let publication_generation = publication
                .clear_dirty_buckets
                .iter()
                .find(|dirty| dirty.bucket_date == rebuild.bucket_date)
                .map(|dirty| dirty.generation);
            stage_long_term_projection_date_publication(
                pool,
                &rebuild.bucket_date,
                &token,
                publication_token,
                publication_generation,
                control,
            )
            .await?;
        }
        clear_long_term_projection_rebuild_members(pool, &token, control).await?;
        rebuild_tokens.push(token.clone());
        if rebuild.interval_segments.is_empty() {
            let (mut transaction, permit) = control.begin(pool).await?;
            sqlx::query(
                "INSERT INTO long_term_projection_bucket_state (bucket_date, interval_baseline_ready) VALUES (?1, 0) ON CONFLICT(bucket_date) DO UPDATE SET interval_baseline_ready = 0, updated_at = datetime('now')",
            )
            .bind(&rebuild.bucket_date)
            .execute(&mut *transaction)
            .await?;
            control.commit(transaction, permit).await?;
        }
        for (batch_index, batch) in rebuild
            .interval_segments
            .chunks(LONG_TERM_PROJECTION_REBUILD_SEGMENT_ROWS)
            .enumerate()
        {
            let (mut transaction, permit) = control.begin(pool).await?;
            if batch_index == 0 {
                sqlx::query(
                    "INSERT INTO long_term_projection_bucket_state (bucket_date, interval_baseline_ready) VALUES (?1, 0) ON CONFLICT(bucket_date) DO UPDATE SET interval_baseline_ready = 0, updated_at = datetime('now')",
                )
                .bind(&rebuild.bucket_date)
                .execute(&mut *transaction)
                .await?;
            }
            for segment in batch {
                sqlx::query(
                    "INSERT INTO long_term_projection_interval_state (invocation_row_id, model_series_key, upstream_series_key, interval_start_ms, interval_end_ms) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(invocation_row_id) DO UPDATE SET model_series_key = excluded.model_series_key, upstream_series_key = excluded.upstream_series_key, interval_start_ms = excluded.interval_start_ms, interval_end_ms = excluded.interval_end_ms",
                )
                .bind(segment.invocation_row_id)
                .bind(&segment.model_series_key)
                .bind(&segment.upstream_series_key)
                .bind(segment.interval_start_ms)
                .bind(segment.interval_end_ms)
                .execute(&mut *transaction)
                .await?;
                sqlx::query(
                    "INSERT OR IGNORE INTO long_term_projection_rebuild_members (rebuild_token, invocation_row_id) VALUES (?1, ?2)",
                )
                .bind(&token)
                .bind(segment.invocation_row_id)
                .execute(&mut *transaction)
                .await?;
                sqlx::query(
                    "DELETE FROM long_term_projection_interval_suppressions WHERE invocation_row_id = ?1 AND bucket_date = ?2",
                )
                .bind(segment.invocation_row_id)
                .bind(&rebuild.bucket_date)
                .execute(&mut *transaction)
                .await?;
            }
            control.commit(transaction, permit).await?;
        }

        loop {
            let (mut transaction, permit) = control.begin(pool).await?;
            let suppressed = sqlx::query(
                r#"
                INSERT OR IGNORE INTO long_term_projection_interval_suppressions (invocation_row_id, bucket_date)
                SELECT candidate.invocation_row_id, ?1
                FROM (
                    SELECT state.invocation_row_id
                    FROM long_term_projection_interval_state state
                    WHERE state.interval_start_ms < ?2
                      AND state.interval_end_ms > ?3
                    UNION
                    SELECT legacy.invocation_row_id
                    FROM long_term_projection_intervals legacy
                    WHERE legacy.bucket_date = ?1
                ) candidate
                WHERE NOT EXISTS (
                    SELECT 1
                    FROM long_term_projection_rebuild_members member
                    WHERE member.rebuild_token = ?4
                      AND member.invocation_row_id = candidate.invocation_row_id
                )
                  AND NOT EXISTS (
                    SELECT 1
                    FROM long_term_projection_interval_suppressions suppressed
                    WHERE suppressed.invocation_row_id = candidate.invocation_row_id
                      AND suppressed.bucket_date = ?1
                  )
                LIMIT ?5
                "#,
            )
            .bind(&rebuild.bucket_date)
            .bind(rebuild.end_epoch * 1_000)
            .bind(rebuild.start_epoch * 1_000)
            .bind(&token)
            .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
            .execute(&mut *transaction)
            .await?
            .rows_affected();
            control.commit(transaction, permit).await?;
            if suppressed == 0 {
                break;
            }
        }
    }

    for rebuild in rebuilds {
        replace_long_term_projection_date_rollups(pool, rebuild, control).await?;
    }

    for token in &rebuild_tokens {
        clear_long_term_projection_rebuild_members(pool, token, control).await?;
    }

    if !publication.publish_state {
        return Ok(());
    }

    // Publishing the token, cursor, and status is a single small transaction. Cleanup only
    // removes already-published indirection, so cancellation cannot expose a staged prefix.
    let (mut transaction, permit) = control.begin(pool).await?;
    if publication.mark_ready {
        let initial_marker = sqlx::query_scalar::<_, Option<String>>(
            "SELECT last_error FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_optional(&mut *transaction)
        .await?
        .flatten();
        if initial_marker.as_deref() == Some(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR) {
            bail!(
                "long-term projection baseline cannot publish over an incomplete initial materialization"
            );
        }
    }
    if let Some(publication_token) = publication.publication_token {
        sqlx::query(
            "INSERT INTO long_term_projection_date_publications (publication_token, published) VALUES (?1, 1) ON CONFLICT(publication_token) DO UPDATE SET published = 1, updated_at = datetime('now')",
        )
        .bind(publication_token)
        .execute(&mut *transaction)
        .await?;
    } else {
        let published_dirty = publication
            .clear_dirty_buckets
            .iter()
            .filter(|dirty| {
                rebuilds
                    .iter()
                    .any(|rebuild| rebuild.bucket_date == dirty.bucket_date)
            })
            .collect::<Vec<_>>();
        if !published_dirty.is_empty() {
            let mut query = QueryBuilder::<Sqlite>::new(
                "DELETE FROM long_term_projection_dirty_buckets WHERE ",
            );
            for (index, dirty) in published_dirty.iter().enumerate() {
                if index > 0 {
                    query.push(" OR ");
                }
                query
                    .push("(bucket_date = ")
                    .push_bind(&dirty.bucket_date)
                    .push(" AND generation = ")
                    .push_bind(dirty.generation)
                    .push(")");
            }
            query.build().execute(&mut *transaction).await?;
        }
    }
    if publication.publication_token.is_none() && !rebuild_tokens.is_empty() {
        let mut query = QueryBuilder::<Sqlite>::new(
            "UPDATE long_term_projection_bucket_state SET active_daily_backup_token = NULL, publication_token = 'cleanup:' || active_daily_backup_token, publication_generation = NULL, updated_at = datetime('now') WHERE ",
        );
        for (index, (rebuild, token)) in rebuilds.iter().zip(&rebuild_tokens).enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("(bucket_date = ")
                .push_bind(&rebuild.bucket_date)
                .push(" AND active_daily_backup_token = ")
                .push_bind(token)
                .push(")");
        }
        query.build().execute(&mut *transaction).await?;
        let mut query = QueryBuilder::<Sqlite>::new(
            "DELETE FROM long_term_projection_daily_backup_claims WHERE ",
        );
        for (index, (rebuild, token)) in rebuilds.iter().zip(&rebuild_tokens).enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("(bucket_date = ")
                .push_bind(&rebuild.bucket_date)
                .push(" AND rebuild_token = ")
                .push_bind(token)
                .push(")");
        }
        query.build().execute(&mut *transaction).await?;
    }
    if let Some(cursor) = publication.next_cursor {
        sqlx::query(
            "INSERT INTO long_term_projection_state (consumer, cursor_row_id, last_flush_at, last_error) VALUES (?1, ?2, datetime('now'), NULL) ON CONFLICT(consumer) DO UPDATE SET cursor_row_id = MAX(long_term_projection_state.cursor_row_id, excluded.cursor_row_id), last_flush_at = excluded.last_flush_at, last_error = NULL, updated_at = datetime('now')",
        )
        .bind(LONG_TERM_PROJECTION_CONSUMER)
        .bind(cursor)
        .execute(&mut *transaction)
        .await?;
    }
    sqlx::query(
        "UPDATE long_term_stats_state SET status = CASE WHEN ?1 OR (?2 AND ?3 AND status = ?4) THEN ?5 ELSE status END, statistics_start_date = CASE WHEN ?6 IS NULL THEN statistics_start_date WHEN statistics_start_date IS NULL OR ?6 < statistics_start_date THEN ?6 ELSE statistics_start_date END, last_error = CASE WHEN ?1 OR (?2 AND ?3 AND status = ?4) THEN NULL ELSE last_error END, updated_at = datetime('now') WHERE id = ?7",
    )
    .bind(publication.mark_ready)
    .bind(repaired_nonempty)
    .bind(publication.publish_state)
    .bind(LONG_TERM_STATUS_EMPTY)
    .bind(LONG_TERM_STATUS_READY)
    .bind(repaired_start_date)
    .bind(LONG_TERM_STATE_ID)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await?;
    if publication.publication_token.is_none() {
        let _ = finish_long_term_projection_backup_cleanup(pool, control).await?;
    }
    Ok(())
}

async fn release_long_term_projection_date_publication(
    pool: &Pool<Sqlite>,
    rebuilds: &[LongTermProjectionDateRebuild],
    clear_dirty_buckets: &[LongTermProjectionDirtyBucket],
    publication_token: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    for rebuild in rebuilds {
        let rebuild_token = format!("long-term-date:{}", rebuild.bucket_date);
        let publication_generation = clear_dirty_buckets
            .iter()
            .find(|dirty| dirty.bucket_date == rebuild.bucket_date)
            .map(|dirty| dirty.generation);
        release_long_term_projection_publication_member(
            pool,
            &rebuild.bucket_date,
            &rebuild_token,
            publication_generation,
            publication_token,
            control,
        )
        .await?;
    }
    let _ = prune_long_term_projection_publications(pool, control).await?;
    Ok(())
}

async fn release_long_term_projection_publication_member(
    pool: &Pool<Sqlite>,
    bucket_date: &str,
    rebuild_token: &str,
    publication_generation: Option<i64>,
    publication_token: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<bool> {
    let (mut transaction, permit) = control.begin(pool).await?;
    let has_newer_dirty = if let Some(publication_generation) = publication_generation {
        sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM long_term_projection_dirty_buckets WHERE bucket_date = ?1 AND generation <> ?2)",
        )
        .bind(bucket_date)
        .bind(publication_generation)
        .fetch_one(&mut *transaction)
        .await?
            != 0
    } else {
        sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM long_term_projection_dirty_buckets WHERE bucket_date = ?1)",
        )
        .bind(bucket_date)
        .fetch_one(&mut *transaction)
        .await?
            != 0
    };
    if has_newer_dirty {
        control.commit(transaction, permit).await?;
        return Ok(false);
    }
    if let Some(publication_generation) = publication_generation {
        sqlx::query(
            "DELETE FROM long_term_projection_dirty_buckets WHERE bucket_date = ?1 AND generation = ?2",
        )
        .bind(bucket_date)
        .bind(publication_generation)
        .execute(&mut *transaction)
        .await?;
    }
    let released = sqlx::query(
        "UPDATE long_term_projection_bucket_state SET active_daily_backup_token = NULL, publication_token = 'cleanup:' || active_daily_backup_token, publication_generation = NULL, updated_at = datetime('now') WHERE bucket_date = ?1 AND active_daily_backup_token = ?2 AND publication_token = ?3",
    )
    .bind(bucket_date)
    .bind(rebuild_token)
    .bind(publication_token)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    if released != 0 {
        sqlx::query(
            "DELETE FROM long_term_projection_daily_backup_claims WHERE bucket_date = ?1 AND rebuild_token = ?2",
        )
        .bind(bucket_date)
        .bind(rebuild_token)
        .execute(&mut *transaction)
        .await?;
    }
    control.commit(transaction, permit).await?;
    if released != 0 {
        control.complete_backup_cleanup_marker();
        control.check()?;
    }
    Ok(released != 0)
}

async fn finish_long_term_projection_publication_cleanup(
    pool: &Pool<Sqlite>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<bool> {
    if finish_long_term_projection_backup_cleanup(pool, control).await? {
        return Ok(true);
    }
    let member = sqlx::query_as::<_, LongTermProjectionPublicationMember>(
        "SELECT state.bucket_date, state.active_daily_backup_token AS rebuild_token, state.publication_token, state.publication_generation FROM long_term_projection_bucket_state state JOIN long_term_projection_date_publications publication ON publication.publication_token = state.publication_token WHERE publication.published = 1 AND state.active_daily_backup_token IS NOT NULL ORDER BY state.updated_at ASC, state.bucket_date ASC LIMIT ?1",
    )
    .bind(1_i64)
    .fetch_optional(pool)
    .await?;
    if let Some(member) = member {
        let _released = release_long_term_projection_publication_member(
            pool,
            &member.bucket_date,
            &member.rebuild_token,
            member.publication_generation,
            &member.publication_token,
            control,
        )
        .await?;
        return Ok(true);
    }
    prune_long_term_projection_publications(pool, control).await
}

async fn prune_long_term_projection_publications(
    pool: &Pool<Sqlite>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<bool> {
    let (mut transaction, permit) = control.begin(pool).await?;
    let deleted = sqlx::query(
        r#"
            DELETE FROM long_term_projection_date_publications
            WHERE rowid IN (
                SELECT publication.rowid
                FROM long_term_projection_date_publications publication
                WHERE NOT EXISTS (
                    SELECT 1
                    FROM long_term_projection_bucket_state state
                    WHERE state.publication_token = publication.publication_token
                )
                ORDER BY publication.updated_at ASC, publication.publication_token ASC
                LIMIT ?1
            )
            "#,
    )
    .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    control.commit(transaction, permit).await?;
    Ok(deleted != 0)
}

async fn rebuild_long_term_projection_date(pool: &Pool<Sqlite>, bucket_date: &str) -> Result<()> {
    let control = LongTermProjectionWriteControl::unrestricted();
    let rebuild = build_long_term_projection_date_rebuild(pool, bucket_date, &control).await?;
    commit_long_term_projection_date_rebuilds(pool, &[rebuild], None, &[], false).await
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
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let unique_paths = file_paths.iter().collect::<HashSet<_>>();
    for file_path in unique_paths {
        let (mut transaction, permit) = control.begin(pool).await?;
        let cleared = sqlx::query(
            "DELETE FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = 'codex_invocations' AND file_path = ?2",
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .bind(file_path)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        control.commit(transaction, permit).await?;
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
            long_term_invocation_archive_safe_start(pool, file_path).await
        }
        "pool_upstream_request_attempts" => {
            long_term_attempt_archive_safe_start(pool, file_path, coverage_end_at).await
        }
        _ => Ok(None),
    }
}

async fn long_term_invocation_archive_safe_start(
    pool: &Pool<Sqlite>,
    file_path: &str,
) -> Result<Option<NaiveDate>> {
    let archive_sha256 = load_long_term_archive_sha256(pool, file_path)
        .await?
        .context("completed invocation archive has no manifest sha256")?;
    let rows = load_long_term_source_timing_rows_from_archive(
        pool,
        file_path,
        &archive_sha256,
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
    let archive_sha256 = load_long_term_archive_sha256_for_dataset(
        pool,
        "pool_upstream_request_attempts",
        file_path,
    )
    .await?
    .context("completed attempt archive has no manifest sha256")?;
    ensure_long_term_archive_source_identity(
        pool,
        "pool_upstream_request_attempts",
        file_path,
        &archive_sha256,
    )
    .await?;
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
    ensure_long_term_archive_source_identity(
        pool,
        "pool_upstream_request_attempts",
        file_path,
        &archive_sha256,
    )
    .await?;
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
    pool: &Pool<Sqlite>,
    file_path: &str,
    archive_sha256: &str,
    read_surface: &'static str,
) -> Result<Vec<LongTermSourceTimingRow>> {
    ensure_long_term_archive_source_identity(pool, "codex_invocations", file_path, archive_sha256)
        .await?;
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
    ensure_long_term_archive_source_identity(pool, "codex_invocations", file_path, archive_sha256)
        .await?;
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
        let archive_sha256 = load_long_term_archive_sha256(pool, archive_path.file_path())
            .await?
            .context("completed invocation archive has no manifest sha256")?;
        ensure_long_term_archive_source_identity(
            pool,
            "codex_invocations",
            archive_path.file_path(),
            &archive_sha256,
        )
        .await?;
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
        ensure_long_term_archive_source_identity(
            pool,
            "codex_invocations",
            archive_path.file_path(),
            &archive_sha256,
        )
        .await?;
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
    let start = parse_long_term_timestamp(occurred_at)?;
    let end_ms = t_total_ms
        .filter(|value| value.is_finite() && *value > 0.0)
        .and_then(|duration_ms| long_term_interval_end_ms(start, duration_ms))
        .unwrap_or(start.epoch_ms);
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
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let (mut transaction, permit) = control.begin(pool).await?;
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
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await
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

async fn mark_long_term_integrity_audit(
    pool: &Pool<Sqlite>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let (mut transaction, permit) = control.begin(pool).await?;
    sqlx::query(
        "UPDATE long_term_stats_state SET last_integrity_audit_at = datetime('now') WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await
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
    let control = LongTermProjectionWriteControl::unrestricted();
    mark_long_term_stats_backfill_preparing_with_control(pool, &control).await
}

async fn mark_long_term_stats_backfill_preparing_with_control(
    pool: &Pool<Sqlite>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    // Preserve error across restart so a persisted integrity queue keeps the next refresh on the
    // verified incremental path. Error without durable rows still transitions to running inside
    // refresh_long_term_stats_once.
    let (mut transaction, permit) = control.begin(pool).await?;
    sqlx::query(
        "UPDATE long_term_stats_state SET status = ?1, updated_at = datetime('now') WHERE id = ?2 AND status NOT IN (?3, ?4)",
    )
    .bind(LONG_TERM_STATUS_PREPARING)
    .bind(LONG_TERM_STATE_ID)
    .bind(LONG_TERM_STATUS_READY)
    .bind(LONG_TERM_STATUS_ERROR)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await
}

pub(crate) fn spawn_long_term_stats_backfill(
    pool: Pool<Sqlite>,
    retention_days: u64,
    shutdown: CancellationToken,
) {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(60));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            if shutdown.is_cancelled() {
                break;
            }
            let state = match load_long_term_state(&pool).await {
                Ok(state) => state,
                Err(error) => {
                    warn!(error = %error, "failed to inspect long-term initial materialization state");
                    tokio::select! {
                        _ = shutdown.cancelled() => break,
                        _ = ticker.tick() => {}
                    }
                    continue;
                }
            };
            let rollups_exist = match long_term_rollups_exist(&pool).await {
                Ok(exists) => exists,
                Err(error) => {
                    warn!(error = %error, "failed to inspect long-term initial materialization rollups");
                    tokio::select! {
                        _ = shutdown.cancelled() => break,
                        _ = ticker.tick() => {}
                    }
                    continue;
                }
            };
            if !long_term_initial_materialization_needed(
                &state.status,
                state.last_error.as_deref(),
                rollups_exist,
            ) {
                break;
            }
            let control = LongTermProjectionWriteControl::background(
                &shutdown,
                crate::db_pressure::global_db_pressure_gate(),
            );
            if let Err(error) =
                mark_long_term_stats_backfill_preparing_with_control(&pool, &control).await
            {
                warn!(error = %error, "failed to mark long-term initial materialization preparing");
            } else {
                if shutdown.is_cancelled() {
                    break;
                }
                if let Err(error) =
                    refresh_long_term_stats_with_control(&pool, retention_days, &control).await
                {
                    if long_term_projection_write_is_deferred(&error) {
                        debug!(error = %error, "long-term initial materialization deferred by database pressure");
                    } else {
                        warn!(error = %error, "long-term initial materialization failed");
                    }
                }
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
    let control = LongTermProjectionWriteControl::unrestricted();
    refresh_long_term_stats_with_control(pool, retention_days, &control).await
}

async fn refresh_long_term_stats_with_control(
    pool: &Pool<Sqlite>,
    retention_days: u64,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let _guard = LONG_TERM_REFRESH_LOCK.lock().await;
    run_long_term_refresh_with_retry(control.shutdown, || {
        refresh_long_term_stats_once(pool, retention_days, control)
    })
    .await
}

async fn persist_long_term_refresh_progress(
    pool: &Pool<Sqlite>,
    processed_rows: i64,
    total_rows: i64,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let (mut transaction, permit) = control.begin(pool).await?;
    sqlx::query(
        "UPDATE long_term_stats_state SET processed_rows = ?1, total_rows = ?2, updated_at = datetime('now') WHERE id = ?3",
    )
    .bind(processed_rows)
    .bind(total_rows)
    .bind(LONG_TERM_STATE_ID)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await
}

async fn run_long_term_refresh_with_retry<T, Operation, OperationFuture>(
    shutdown: Option<&CancellationToken>,
    operation: Operation,
) -> Result<T>
where
    Operation: FnMut() -> OperationFuture,
    OperationFuture: Future<Output = Result<T>>,
{
    run_long_term_refresh_with_retry_delays(
        shutdown,
        operation,
        &LONG_TERM_REFRESH_LOCK_RETRY_DELAYS,
    )
    .await
}

async fn run_long_term_refresh_with_retry_delays<T, Operation, OperationFuture>(
    shutdown: Option<&CancellationToken>,
    mut operation: Operation,
    retry_delays: &[Duration],
) -> Result<T>
where
    Operation: FnMut() -> OperationFuture,
    OperationFuture: Future<Output = Result<T>>,
{
    for (attempt, delay) in retry_delays.iter().enumerate() {
        if shutdown.is_some_and(CancellationToken::is_cancelled) {
            bail!("long-term stats refresh cancelled before SQLite lock retry");
        }
        match operation().await {
            Ok(value) => return Ok(value),
            Err(error) if crate::is_sqlite_lock_error(&error) => {
                warn!(
                    attempt = attempt + 1,
                    retry_after_ms = delay.as_millis(),
                    error = %error,
                    "long-term stats refresh hit a SQLite lock; retrying"
                );
                if let Some(shutdown) = shutdown {
                    tokio::select! {
                        _ = shutdown.cancelled() => bail!("long-term stats refresh cancelled during SQLite lock retry"),
                        _ = sleep(*delay) => {}
                    }
                } else {
                    sleep(*delay).await;
                }
            }
            Err(error) => return Err(error),
        }
    }
    if shutdown.is_some_and(CancellationToken::is_cancelled) {
        bail!("long-term stats refresh cancelled before SQLite lock retry");
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
        // Durable daily backups keep the ready read model on the prior complete date until its
        // bounded replacement is fully written and published.
        (LONG_TERM_STATUS_READY, true)
    }
}

fn long_term_refresh_pending_marker(
    was_ready: bool,
    starting_status: &str,
) -> Option<&'static str> {
    (!was_ready && starting_status != LONG_TERM_STATUS_ERROR)
        .then_some(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR)
}

async fn refresh_long_term_stats_once(
    pool: &Pool<Sqlite>,
    retention_days: u64,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let refresh_started_at = format_utc_iso(Utc::now());
    let state_snapshot =
        sqlx::query_as::<_, (Option<String>, Option<String>, Option<String>, Option<String>)>(
            "SELECT status, statistics_start_date, integrity_source_start_date, last_error FROM long_term_stats_state WHERE id = ?1",
        )
    .bind(LONG_TERM_STATE_ID)
    .fetch_optional(pool)
    .await?;
    let was_ready =
        state_snapshot
            .as_ref()
            .is_some_and(|(status, statistics_start_date, _, last_error)| {
                status.as_deref().is_some_and(|status| {
                    matches!(status, LONG_TERM_STATUS_READY | LONG_TERM_STATUS_EMPTY)
                    // An error after the final publication still has a durable baseline. Keep
                    // its replacement incremental so a deferred repair retains its retry
                    // backoff. A failed initial refresh retains its explicit pending marker,
                    // even if it had reached a provisional start date before an archive read
                    // failed, so recovery replays every source archive.
                    || (status == LONG_TERM_STATUS_ERROR
                        && statistics_start_date.is_some()
                        && last_error.as_deref()
                            != Some(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR))
                })
            });
    let today = Utc::now().with_timezone(&Shanghai).date_naive();
    let retention_start = today - ChronoDuration::days(retention_days.max(366) as i64 - 1);
    let reconstructable_start = long_term_reconstructable_start(
        retention_start,
        state_snapshot
            .as_ref()
            .and_then(|(_, statistics_start_date, _, _)| statistics_start_date.as_deref()),
        state_snapshot
            .as_ref()
            .and_then(|(_, _, integrity_source_start_date, _)| {
                integrity_source_start_date.as_deref()
            }),
    );
    let has_pending_integrity_repairs = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM long_term_stats_repair_queue WHERE stats_date >= ?1)",
    )
    .bind(reconstructable_start.to_string())
    .fetch_one(pool)
    .await?
        != 0;
    let preserves_prior_error = state_snapshot.as_ref().is_some_and(|(status, _, _, _)| {
        status
            .as_deref()
            .is_some_and(|status| status == LONG_TERM_STATUS_ERROR)
    });
    let (starting_status, clear_last_error) = long_term_refresh_start_state(
        was_ready,
        has_pending_integrity_repairs || preserves_prior_error,
    );
    let pending_marker = long_term_refresh_pending_marker(was_ready, starting_status);
    let (mut transaction, permit) = control.begin(pool).await?;
    if let Some(pending_marker) = pending_marker {
        // A bounded replacement may be interrupted after committing only one batch. Persist a
        // distinct marker before those writes so startup and the P2 cursor never mistake a
        // partial durable prefix for a complete baseline.
        sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, last_error = ?2, updated_at = datetime('now') WHERE id = ?3",
        )
        .bind(starting_status)
        .bind(pending_marker)
        .bind(LONG_TERM_STATE_ID)
        .execute(&mut *transaction)
        .await?;
    } else if clear_last_error {
        sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, last_error = NULL, updated_at = datetime('now') WHERE id = ?2",
        )
        .bind(starting_status)
        .bind(LONG_TERM_STATE_ID)
        .execute(&mut *transaction)
        .await?;
    } else {
        // Keep known-bad materialized data hidden throughout a repair attempt. The final
        // replacement transaction is the only path that clears the queue and restores ready.
        sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, updated_at = datetime('now') WHERE id = ?2",
        )
        .bind(starting_status)
        .bind(LONG_TERM_STATE_ID)
        .execute(&mut *transaction)
        .await?;
    }
    control.commit(transaction, permit).await?;

    let result = refresh_long_term_stats_inner(
        pool,
        retention_days,
        !was_ready,
        &refresh_started_at,
        control,
    )
    .await;
    if let Err(err) = &result
        && let Ok((mut transaction, permit)) = control.begin(pool).await
    {
        let _ = sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, last_error = ?2, updated_at = datetime('now') WHERE id = ?3",
        )
        .bind(LONG_TERM_STATUS_ERROR)
        .bind(err.to_string())
        .bind(LONG_TERM_STATE_ID)
        .execute(&mut *transaction)
        .await;
        let _ = control.commit(transaction, permit).await;
    }
    result
}

async fn refresh_long_term_stats_inner(
    pool: &Pool<Sqlite>,
    retention_days: u64,
    initial_materialization: bool,
    refresh_started_at: &str,
    control: &LongTermProjectionWriteControl<'_>,
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
    let ready_state = !initial_materialization && !legacy_model_keys;
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
    if ready_state || integrity_audit_due || terminal_proof_reconciliation_incomplete {
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
        let (mut transaction, permit) = control.begin(pool).await?;
        sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, last_error = ?2, updated_at = datetime('now') WHERE id = ?3 AND NOT (status = ?4 AND datetime(updated_at) > datetime(?5))",
        )
        .bind(LONG_TERM_STATUS_ERROR)
        .bind(LONG_TERM_TERMINAL_PROOF_UNAVAILABLE_ERROR)
        .bind(LONG_TERM_STATE_ID)
        .bind(LONG_TERM_STATUS_PREPARING)
        .bind(refresh_started_at)
        .execute(&mut *transaction)
        .await?;
        control.commit(transaction, permit).await?;
    }
    if !unavailable_reconciliation_archive_paths.is_empty() {
        clear_long_term_invocation_replay_markers_for_unavailable_sources(
            pool,
            &unavailable_reconciliation_archive_paths,
            control,
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
                // The archive workload has not been enumerated yet during a full rebuild, so
                // keep the total explicitly unknown instead of presenting a false completion
                // ratio to the preparation UI.
                persist_long_term_refresh_progress(pool, processed_rows_count, 0, control).await?;
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
    let stale_marker_cleanup = delete_long_term_refresh_replay_markers_with_control(
        pool,
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE rowid IN (
            SELECT replay.rowid
            FROM hourly_rollup_archive_replay replay
            WHERE replay.target = ?1
              AND replay.dataset = 'codex_invocations'
              AND EXISTS (
                  SELECT 1
                  FROM archive_batches batches
                  WHERE batches.dataset = 'codex_invocations'
                    AND batches.file_path = replay.file_path
                    AND datetime(batches.created_at) > datetime(replay.replayed_at)
              )
            LIMIT ?2
        )
        "#,
        &[LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET.to_string()],
        control,
    )
    .await;
    if let Err(error) = stale_marker_cleanup
        && !error.to_string().contains("no such table")
    {
        return Err(error);
    }
    let replayed_archive_files = if !ready_state {
        HashSet::new()
    } else {
        match sqlx::query_as::<_, (String, Option<String>)>(
            r#"
        SELECT replay.file_path, replay.source_identity
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
            Ok(rows) => rows
                .into_iter()
                .filter(|(file_path, source_identity)| {
                    source_identity.as_deref().is_some_and(|source_identity| {
                        long_term_archive_file_identity(file_path).ok().as_deref()
                            == Some(source_identity)
                    })
                })
                .map(|(file_path, _)| file_path)
                .collect::<HashSet<_>>(),
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
        let archive_sha256_before_open = match crate::maintenance::sha256_hex_file(
            std::path::Path::new(archive_path.file_path()),
        ) {
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
                warn!(error = %error, file_path = archive_path.file_path(), "long-term stats archive identity read failed");
                continue;
            }
        };
        let archive_manifest_sha256 =
            load_long_term_archive_sha256(pool, archive_path.file_path()).await?;
        if !long_term_archive_scan_identity_matches_manifest(
            &archive_sha256_before_open,
            Some(&archive_sha256_before_open),
            archive_manifest_sha256.as_deref(),
        ) {
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
            warn!(
                file_path = archive_path.file_path(),
                "long-term stats archive does not match its completed manifest; clearing its replay marker for retry"
            );
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
                let archive_sha256_after_read = crate::maintenance::sha256_hex_file(
                    std::path::Path::new(archive_path.file_path()),
                );
                let archive_manifest_sha256 =
                    load_long_term_archive_sha256(pool, archive_path.file_path()).await?;
                if long_term_archive_scan_identity_matches_manifest(
                    &archive_sha256_before_open,
                    archive_sha256_after_read.as_deref().ok(),
                    archive_manifest_sha256.as_deref(),
                ) {
                    archive_markers.push((
                        archive_path.file_path().to_string(),
                        archive_sha256_before_open,
                    ));
                } else {
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
                    warn!(
                        file_path = archive_path.file_path(),
                        error = ?archive_sha256_after_read.as_ref().err(),
                        "long-term stats archive changed while it was scanned; clearing its replay marker for retry"
                    );
                }
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
        persist_long_term_refresh_progress(pool, 0, total_rows, control).await?;
    }
    for (index, row) in rows.iter().enumerate() {
        let mut row = row.clone();
        hydrate_long_term_account_identity(&mut row, &account_identities);
        accumulate_long_term_invocation(&row, &mut hourly, &mut daily, &mut statistics_start_date);
        if ready_state && ((index + 1) % 256 == 0 || index + 1 == rows.len()) {
            persist_long_term_refresh_progress(pool, (index + 1) as i64, total_rows, control)
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
        enqueue_long_term_integrity_mismatch(pool, &mismatch, control).await?;
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
            enqueue_long_term_integrity_mismatch(pool, &mismatch, control).await?;
        }
        mark_long_term_integrity_audit(pool, control).await?;
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
            let date_string = date.to_string();
            match load_long_term_integrity_oracle(pool, *date).await? {
                Some(oracle) => {
                    let candidate_is_empty = !daily
                        .keys()
                        .any(|(bucket_date, _, _)| bucket_date == &date_string)
                        && !hourly.keys().any(|(bucket_start_epoch, _, _)| {
                            long_term_bucket_date(*bucket_start_epoch) == Some(*date)
                        });
                    if scheduled_repair_date == Some(*date) && candidate_is_empty {
                        // A durable repair queue represents an explicitly invalidated date. A
                        // complete empty source is a valid replacement even when the prior
                        // canonical proof still contains the stale pre-repair totals.
                        completed_integrity_repairs.insert(*date);
                    } else if let Some(mismatch) = long_term_integrity_mismatch(
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
                None if initial_materialization
                    && !archive_read_failed
                    && !terminal_proof_reconciliation_incomplete
                    && *date >= reconstructable_start =>
                {
                    // The initial pass reads every retained live and archive source before
                    // publishing. That complete snapshot is sufficient bootstrap evidence until
                    // the canonical hourly proof is reconciled; retired source prefixes remain
                    // outside the reconstructable window and cannot take this path.
                    if scheduled_repair_date == Some(*date) {
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
            let archive_sha256 = load_long_term_archive_sha256(pool, archive_path.file_path())
                .await?
                .context("completed invocation archive has no manifest sha256")?;
            ensure_long_term_archive_source_identity(
                pool,
                "codex_invocations",
                archive_path.file_path(),
                &archive_sha256,
            )
            .await?;
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
            ensure_long_term_archive_source_identity(
                pool,
                "codex_invocations",
                archive_path.file_path(),
                &archive_sha256,
            )
            .await?;
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

    apply_long_term_refresh_rollups_with_control(
        pool,
        LongTermRefreshRollupInput {
            hourly: &hourly,
            daily: &daily,
            recomputed_dates: &recomputed_dates,
            retention_start,
            integrity_repair_failures: &integrity_repair_failures,
            completed_integrity_repairs: &completed_integrity_repairs,
            reconstructable_start,
            statistics_start_date: statistics_start_date.as_deref(),
            initial_materialization,
            processed_rows_count: if ready_state {
                rows.len() as i64
            } else {
                processed_rows_count
            },
            source_rows_empty: rows.is_empty(),
            archive_read_failed,
            terminal_proof_reconciliation_incomplete,
            archive_markers: &archive_markers,
            failed_archive_paths: &failed_archive_paths,
            clear_all_attempt_markers,
            failed_archive_ranges: &failed_archive_ranges,
            attempt_archive_markers: &attempt_archive_markers,
        },
        control,
    )
    .await
}

async fn delete_initial_long_term_rollups_for_date(
    pool: &Pool<Sqlite>,
    date: NaiveDate,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let Some((start_epoch, end_epoch)) = long_term_day_epoch_bounds(date) else {
        return Ok(());
    };
    for statement in [
        "DELETE FROM long_term_usage_daily WHERE rowid IN (SELECT rowid FROM long_term_usage_daily WHERE stats_date = ?1 LIMIT ?2)",
        "DELETE FROM long_term_usage_hourly WHERE rowid IN (SELECT rowid FROM long_term_usage_hourly WHERE bucket_start_epoch >= ?1 AND bucket_start_epoch < ?2 LIMIT ?3)",
    ] {
        loop {
            let (mut transaction, permit) = control.begin(pool).await?;
            let mut query = sqlx::query(statement);
            if statement.contains("bucket_start_epoch") {
                query = query
                    .bind(start_epoch)
                    .bind(end_epoch)
                    .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64);
            } else {
                query = query
                    .bind(date.to_string())
                    .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64);
            }
            let deleted = query.execute(&mut *transaction).await?.rows_affected();
            control.commit(transaction, permit).await?;
            if deleted == 0 {
                break;
            }
        }
    }
    Ok(())
}

struct LongTermRefreshRollupInput<'a> {
    hourly: &'a HashMap<(i64, String, String), LongTermBucket>,
    daily: &'a HashMap<(String, String, String), LongTermBucket>,
    recomputed_dates: &'a HashSet<NaiveDate>,
    retention_start: NaiveDate,
    integrity_repair_failures: &'a [LongTermIntegrityMismatch],
    completed_integrity_repairs: &'a HashSet<NaiveDate>,
    reconstructable_start: NaiveDate,
    statistics_start_date: Option<&'a str>,
    initial_materialization: bool,
    processed_rows_count: i64,
    source_rows_empty: bool,
    archive_read_failed: bool,
    terminal_proof_reconciliation_incomplete: bool,
    archive_markers: &'a [(String, String)],
    failed_archive_paths: &'a HashSet<String>,
    clear_all_attempt_markers: bool,
    failed_archive_ranges: &'a [(String, String)],
    attempt_archive_markers: &'a HashSet<(String, String)>,
}

struct LongTermRefreshArchiveMarkers<'a> {
    archive_markers: &'a [(String, String)],
    archive_read_failed: bool,
    failed_archive_paths: &'a HashSet<String>,
    clear_all_attempt_markers: bool,
    failed_archive_ranges: &'a [(String, String)],
    attempt_archive_markers: &'a HashSet<(String, String)>,
}

async fn apply_long_term_refresh_rollups_with_control(
    pool: &Pool<Sqlite>,
    input: LongTermRefreshRollupInput<'_>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let LongTermRefreshRollupInput {
        hourly,
        daily,
        recomputed_dates,
        retention_start,
        integrity_repair_failures,
        completed_integrity_repairs,
        reconstructable_start,
        statistics_start_date,
        initial_materialization,
        processed_rows_count,
        source_rows_empty,
        archive_read_failed,
        terminal_proof_reconciliation_incomplete,
        archive_markers,
        failed_archive_paths,
        clear_all_attempt_markers,
        failed_archive_ranges,
        attempt_archive_markers,
    } = input;
    let has_persisted_daily_rows =
        sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM long_term_usage_daily LIMIT 1)")
            .fetch_one(pool)
            .await?
            != 0;
    // A pending initial materialization has no publishable baseline. If its only partial prefix
    // is retried against a successfully empty source, clear it before reporting `empty` rather
    // than promoting stale rows to a completed initial snapshot.
    let clears_pending_empty_initial_materialization = initial_materialization
        && source_rows_empty
        && !archive_read_failed
        && !terminal_proof_reconciliation_incomplete;
    let mut refresh_backups = Vec::with_capacity(recomputed_dates.len());
    for date in recomputed_dates {
        let bucket_date = date.to_string();
        // P2 and the initial refresher intentionally share the durable owner for a date. If a
        // P2 snapshot won the race just before the initial marker was persisted, the refresher
        // can finish and release that same last-good backup instead of being stranded by it.
        let rebuild_token = format!("long-term-date:{bucket_date}");
        // The full refresher also writes in bounded transactions. Keep the same durable daily
        // snapshot used by a P2 date rebuild so pressure or shutdown cannot leave a deleted
        // prefix as the only recoverable state.
        ensure_long_term_projection_daily_backup_for_date(
            pool,
            &bucket_date,
            &rebuild_token,
            false,
            control,
        )
        .await?;
        refresh_backups.push((bucket_date, rebuild_token));
    }
    if recomputed_dates.is_empty()
        && (!has_persisted_daily_rows || clears_pending_empty_initial_materialization)
    {
        for table in ["long_term_usage_hourly", "long_term_usage_daily"] {
            loop {
                let (mut transaction, permit) = control.begin(pool).await?;
                let deleted = sqlx::query(&format!(
                    "DELETE FROM {table} WHERE rowid IN (SELECT rowid FROM {table} LIMIT ?1)"
                ))
                .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
                .execute(&mut *transaction)
                .await?
                .rows_affected();
                control.commit(transaction, permit).await?;
                if deleted == 0 {
                    break;
                }
            }
        }
    } else {
        for date in recomputed_dates {
            delete_initial_long_term_rollups_for_date(pool, *date, control).await?;
        }
    }

    let retention_start_epoch = retention_start
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Shanghai.from_local_datetime(&value).single())
        .map(|value| value.timestamp())
        .unwrap_or(i64::MIN);
    loop {
        let (mut transaction, permit) = control.begin(pool).await?;
        let deleted = sqlx::query(
            "DELETE FROM long_term_usage_hourly WHERE rowid IN (SELECT rowid FROM long_term_usage_hourly WHERE bucket_start_epoch < ?1 LIMIT ?2)",
        )
        .bind(retention_start_epoch)
        .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        control.commit(transaction, permit).await?;
        if deleted == 0 {
            break;
        }
    }

    let hourly = hourly
        .values()
        .filter(|bucket| {
            Shanghai
                .timestamp_opt(bucket.bucket_start_epoch, 0)
                .single()
                .is_some_and(|value| value.date_naive() >= retention_start)
        })
        .collect::<Vec<_>>();
    for batch in hourly.chunks(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS) {
        let (mut transaction, permit) = control.begin(pool).await?;
        for bucket in batch {
            insert_long_term_hourly(&mut transaction, bucket).await?;
        }
        control.commit(transaction, permit).await?;
    }
    let daily = daily.values().collect::<Vec<_>>();
    for batch in daily.chunks(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS) {
        let (mut transaction, permit) = control.begin(pool).await?;
        for bucket in batch {
            insert_long_term_daily(&mut transaction, bucket).await?;
        }
        control.commit(transaction, permit).await?;
    }
    let mut retry_dates = HashSet::new();
    for mismatch in integrity_repair_failures
        .iter()
        .filter(|mismatch| retry_dates.insert(mismatch.date))
    {
        let (mut transaction, permit) = control.begin(pool).await?;
        schedule_long_term_repair_retry(&mut transaction, mismatch).await?;
        control.commit(transaction, permit).await?;
    }
    // The planner selects one due repair date per refresh. Keep this final publication write
    // bounded even if a future caller accidentally changes that scheduling contract.
    if completed_integrity_repairs.len() > 1 {
        bail!("long-term refresh may complete at most one integrity repair per publication");
    }
    // A completed repair is not durable until its candidate has been published. Account for it
    // while choosing the public state, but retain its queue entry until the final publication
    // transaction so cancellation cannot strand an unpublished backup without a retry path.
    let completed_integrity_repair_dates = completed_integrity_repairs
        .iter()
        .filter(|date| **date >= reconstructable_start)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let mut pending_integrity_repairs_query = QueryBuilder::<Sqlite>::new(
        "SELECT COUNT(*) FROM long_term_stats_repair_queue WHERE stats_date >= ",
    );
    pending_integrity_repairs_query.push_bind(reconstructable_start.to_string());
    if !completed_integrity_repair_dates.is_empty() {
        pending_integrity_repairs_query.push(" AND stats_date NOT IN (");
        let mut dates = pending_integrity_repairs_query.separated(", ");
        for date in &completed_integrity_repair_dates {
            dates.push_bind(date);
        }
        dates.push_unseparated(")");
    }
    let pending_integrity_repairs = pending_integrity_repairs_query
        .build_query_scalar::<i64>()
        .fetch_one(pool)
        .await?;
    let status = if pending_integrity_repairs > 0
        || archive_read_failed
        || terminal_proof_reconciliation_incomplete
    {
        LONG_TERM_STATUS_ERROR
    } else if source_rows_empty
        && daily.is_empty()
        && (!has_persisted_daily_rows || clears_pending_empty_initial_materialization)
    {
        LONG_TERM_STATUS_EMPTY
    } else {
        LONG_TERM_STATUS_READY
    };
    let last_error = if status == LONG_TERM_STATUS_ERROR && initial_materialization {
        Some(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR.to_string())
    } else if terminal_proof_reconciliation_incomplete {
        Some(LONG_TERM_TERMINAL_PROOF_UNAVAILABLE_ERROR.to_string())
    } else if pending_integrity_repairs > 0 {
        Some(format!(
            "long-term integrity repair pending for {pending_integrity_repairs} date(s)"
        ))
    } else if archive_read_failed {
        Some("one or more invocation archives could not be materialized".to_string())
    } else {
        None
    };
    control.complete_integrity_repairs();
    control.check()?;
    // Stage every replacement behind one publication token. The final transaction below flips
    // this token and the public state together, so cancellation cannot expose a mixed refresh.
    let refresh_publication_token =
        (!refresh_backups.is_empty()).then(next_long_term_projection_publication_token);
    if let Some(publication_token) = refresh_publication_token.as_deref() {
        for (bucket_date, rebuild_token) in &refresh_backups {
            stage_long_term_projection_date_publication(
                pool,
                bucket_date,
                rebuild_token,
                publication_token,
                None,
                control,
            )
            .await?;
        }
    }
    let (mut transaction, permit) = control.begin(pool).await?;
    if let Some(publication_token) = refresh_publication_token.as_deref() {
        sqlx::query(
            "INSERT INTO long_term_projection_date_publications (publication_token, published) VALUES (?1, 1) ON CONFLICT(publication_token) DO UPDATE SET published = 1, updated_at = datetime('now')",
        )
        .bind(publication_token)
        .execute(&mut *transaction)
        .await?;
    }
    for date in completed_integrity_repairs {
        sqlx::query("DELETE FROM long_term_stats_repair_queue WHERE stats_date = ?1")
            .bind(date.to_string())
            .execute(&mut *transaction)
            .await?;
    }
    sqlx::query(
        "UPDATE long_term_stats_state SET status = ?1, statistics_start_date = ?2, processed_rows = ?3, total_rows = ?3, last_error = ?4, updated_at = datetime('now') WHERE id = ?5",
    )
    .bind(status)
    .bind(statistics_start_date)
    .bind(processed_rows_count)
    .bind(last_error)
    .bind(LONG_TERM_STATE_ID)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await?;
    for date in completed_integrity_repairs {
        info!(stats_date = %date, "long-term stats integrity repair completed");
    }
    control.complete_refresh_publication();
    control.check()?;
    if let Some(publication_token) = refresh_publication_token.as_deref() {
        for (bucket_date, rebuild_token) in &refresh_backups {
            let _released = release_long_term_projection_publication_member(
                pool,
                bucket_date,
                rebuild_token,
                None,
                publication_token,
                control,
            )
            .await?;
        }
        prune_long_term_projection_publications(pool, control).await?;
    }
    // A retry may safely rebuild an already-published date. It cannot safely skip an archive
    // while its replay marker exists but the replacement is still behind publication cleanup.
    persist_long_term_refresh_archive_markers_with_control(
        pool,
        LongTermRefreshArchiveMarkers {
            archive_markers,
            archive_read_failed,
            failed_archive_paths,
            clear_all_attempt_markers,
            failed_archive_ranges,
            attempt_archive_markers,
        },
        control,
    )
    .await?;
    Ok(())
}

async fn persist_long_term_refresh_archive_markers_with_control(
    pool: &Pool<Sqlite>,
    markers: LongTermRefreshArchiveMarkers<'_>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let LongTermRefreshArchiveMarkers {
        archive_markers,
        archive_read_failed,
        failed_archive_paths,
        clear_all_attempt_markers,
        failed_archive_ranges,
        attempt_archive_markers,
    } = markers;
    for (file_path, archive_sha256) in archive_markers {
        persist_long_term_refresh_archive_marker_with_control(
            pool,
            "codex_invocations",
            file_path,
            archive_sha256,
            control,
        )
        .await?;
    }

    if archive_read_failed {
        for failed_archive_path in failed_archive_paths {
            let (mut transaction, permit) = control.begin(pool).await?;
            sqlx::query(
                "DELETE FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = 'codex_invocations' AND file_path = ?2",
            )
            .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
            .bind(failed_archive_path)
            .execute(&mut *transaction)
            .await?;
            control.commit(transaction, permit).await?;
        }
        if clear_all_attempt_markers {
            delete_long_term_refresh_replay_markers_with_control(
                pool,
                "DELETE FROM hourly_rollup_archive_replay WHERE rowid IN (SELECT rowid FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = 'pool_upstream_request_attempts' LIMIT ?2)",
                &[LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET.to_string()],
                control,
            )
            .await?;
        } else {
            for (failed_start, failed_end) in failed_archive_ranges {
                delete_long_term_refresh_replay_markers_with_control(
                    pool,
                    r#"
                    DELETE FROM hourly_rollup_archive_replay
                    WHERE rowid IN (
                        SELECT replay.rowid
                        FROM hourly_rollup_archive_replay replay
                        WHERE replay.target = ?1
                          AND replay.dataset = 'pool_upstream_request_attempts'
                          AND EXISTS (
                              SELECT 1
                              FROM archive_batches attempts
                              WHERE attempts.dataset = 'pool_upstream_request_attempts'
                                AND attempts.file_path = replay.file_path
                                AND (attempts.coverage_end_at IS NULL OR attempts.coverage_end_at >= ?2)
                                AND (attempts.coverage_start_at IS NULL OR attempts.coverage_start_at <= ?3)
                          )
                        LIMIT ?4
                    )
                    "#,
                    &[
                        LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET.to_string(),
                        failed_start.clone(),
                        failed_end.clone(),
                    ],
                    control,
                )
                .await?;
            }
        }
    } else {
        for (file_path, archive_sha256) in attempt_archive_markers {
            persist_long_term_refresh_archive_marker_with_control(
                pool,
                "pool_upstream_request_attempts",
                file_path,
                archive_sha256,
                control,
            )
            .await?;
        }
    }
    Ok(())
}

async fn persist_long_term_refresh_archive_marker_with_control(
    pool: &Pool<Sqlite>,
    dataset: &str,
    file_path: &str,
    archive_sha256: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let (mut transaction, permit) = control.begin(pool).await?;
    let result = sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256, source_identity, replayed_at)
        SELECT ?1, ?2, ?3, ?4, NULL, strftime('%Y-%m-%d %H:%M:%f', 'now')
        WHERE EXISTS (
            SELECT 1
            FROM archive_batches
            WHERE dataset = ?2
              AND status = 'completed'
              AND file_path = ?3
              AND sha256 = ?4
        )
        ON CONFLICT(target, dataset, file_path) DO UPDATE SET
            archive_sha256 = excluded.archive_sha256,
            source_identity = NULL,
            replayed_at = strftime('%Y-%m-%d %H:%M:%f', 'now')
        "#,
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(dataset)
    .bind(file_path)
    .bind(archive_sha256)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await?;
    if result.rows_affected() != 1 {
        delete_long_term_refresh_archive_marker_with_control(pool, dataset, file_path, control)
            .await?;
        bail!(
            "long-term archive manifest changed before its replay marker could be persisted: {file_path}"
        );
    }
    let source_identity_before = match long_term_archive_file_identity(file_path) {
        Ok(source_identity) => source_identity,
        Err(error) => {
            delete_long_term_refresh_archive_marker_with_control(pool, dataset, file_path, control)
                .await?;
            return Err(error.context(format!(
                "long-term archive source changed before its replay marker could be persisted: {file_path}"
            )));
        }
    };
    if let Err(error) =
        ensure_long_term_archive_source_identity(pool, dataset, file_path, archive_sha256).await
    {
        delete_long_term_refresh_archive_marker_with_control(pool, dataset, file_path, control)
            .await?;
        return Err(error.context(format!(
            "long-term archive source changed before its replay marker could be persisted: {file_path}"
        )));
    }
    let source_identity_after = match long_term_archive_file_identity(file_path) {
        Ok(source_identity) => source_identity,
        Err(error) => {
            delete_long_term_refresh_archive_marker_with_control(pool, dataset, file_path, control)
                .await?;
            return Err(error.context(format!(
                "long-term archive source changed before its replay marker could be persisted: {file_path}"
            )));
        }
    };
    if source_identity_before != source_identity_after {
        delete_long_term_refresh_archive_marker_with_control(pool, dataset, file_path, control)
            .await?;
        bail!(
            "long-term archive source changed before its replay marker could be persisted: {file_path}"
        );
    }
    let (mut transaction, permit) = control.begin(pool).await?;
    let result = sqlx::query(
        r#"
        UPDATE hourly_rollup_archive_replay
        SET source_identity = ?1,
            replayed_at = strftime('%Y-%m-%d %H:%M:%f', 'now')
        WHERE target = ?2
          AND dataset = ?3
          AND file_path = ?4
          AND archive_sha256 = ?5
          AND EXISTS (
              SELECT 1
              FROM archive_batches
              WHERE dataset = ?3
                AND status = 'completed'
                AND file_path = ?4
                AND sha256 = ?5
          )
        "#,
    )
    .bind(&source_identity_after)
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(dataset)
    .bind(file_path)
    .bind(archive_sha256)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await?;
    if result.rows_affected() != 1 {
        delete_long_term_refresh_archive_marker_with_control(pool, dataset, file_path, control)
            .await?;
        bail!(
            "long-term archive manifest changed before its replay marker could be finalized: {file_path}"
        );
    }
    Ok(())
}

async fn delete_long_term_refresh_archive_marker_with_control(
    pool: &Pool<Sqlite>,
    dataset: &str,
    file_path: &str,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let (mut transaction, permit) = control.begin(pool).await?;
    sqlx::query(
        "DELETE FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(dataset)
    .bind(file_path)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await
}

async fn delete_long_term_refresh_replay_markers_with_control(
    pool: &Pool<Sqlite>,
    sql: &str,
    bindings: &[String],
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    loop {
        let (mut transaction, permit) = control.begin(pool).await?;
        let mut query = sqlx::query(sql);
        for binding in bindings {
            query = query.bind(binding);
        }
        let deleted = query
            .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
            .execute(&mut *transaction)
            .await?
            .rows_affected();
        control.commit(transaction, permit).await?;
        if deleted == 0 {
            return Ok(());
        }
    }
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
    let Some(start) = parse_long_term_timestamp(&row.occurred_at) else {
        return;
    };
    let start_ms = start.epoch_ms;
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
        .and_then(|value| long_term_interval_end_ms(start, value));
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

fn collect_long_term_projection_interval_segments(
    hourly: &HashMap<(i64, String, String), LongTermBucket>,
    daily: &HashMap<(String, String, String), LongTermBucket>,
    invocation_row_id: i64,
) -> Vec<LongTermProjectionIntervalSegment> {
    let mut interval_start_ms = None;
    let mut interval_end_ms = None;
    for bucket in hourly
        .values()
        .filter(|bucket| bucket.dimension == "overall")
    {
        for &(start, end) in &bucket.accumulator.intervals {
            interval_start_ms =
                Some(interval_start_ms.map_or(start, |current: i64| current.min(start)));
            interval_end_ms = Some(interval_end_ms.map_or(end, |current: i64| current.max(end)));
        }
    }
    let (Some(interval_start_ms), Some(interval_end_ms)) = (interval_start_ms, interval_end_ms)
    else {
        return Vec::new();
    };
    let model_series_key = daily
        .values()
        .find(|bucket| bucket.dimension == "model")
        .map(|bucket| bucket.series_key.clone())
        .unwrap_or_else(|| LONG_TERM_OTHER_KEY.to_string());
    let upstream_series_key = daily
        .values()
        .find(|bucket| bucket.dimension == "upstream")
        .map(|bucket| bucket.series_key.clone())
        .unwrap_or_else(|| LONG_TERM_OTHER_KEY.to_string());
    vec![LongTermProjectionIntervalSegment {
        invocation_row_id,
        model_series_key,
        upstream_series_key,
        interval_start_ms,
        interval_end_ms,
    }]
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
    let requested_dates = dates
        .iter()
        .filter_map(|date| {
            NaiveDate::parse_from_str(date, "%Y-%m-%d")
                .ok()
                .map(|value| (date, value))
        })
        .collect::<Vec<_>>();
    if requested_dates.is_empty() {
        return Ok(HashMap::new());
    }
    let mut suppressed_builder = QueryBuilder::<Sqlite>::new(
        "SELECT invocation_row_id, bucket_date FROM long_term_projection_interval_suppressions WHERE bucket_date IN (",
    );
    let mut suppressed_dates = suppressed_builder.separated(", ");
    for (date, _) in &requested_dates {
        suppressed_dates.push_bind(*date);
    }
    suppressed_dates.push_unseparated(")");
    let suppressed = suppressed_builder
        .build_query_as::<(i64, String)>()
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect::<HashSet<_>>();
    let first_start_ms = requested_dates
        .iter()
        .filter_map(|(_, date)| long_term_day_epoch_bounds(*date).map(|(start, _)| start * 1_000))
        .min()
        .context("invalid long-term projection interval date range")?;
    let last_end_ms = requested_dates
        .iter()
        .filter_map(|(_, date)| long_term_day_epoch_bounds(*date).map(|(_, end)| end * 1_000))
        .max()
        .context("invalid long-term projection interval date range")?;
    let canonical_rows = sqlx::query_as::<_, LongTermProjectionIntervalStateRow>(
        "SELECT invocation_row_id, model_series_key, upstream_series_key, interval_start_ms, interval_end_ms FROM long_term_projection_interval_state WHERE interval_start_ms < ?1 AND interval_end_ms > ?2",
    )
    .bind(last_end_ms)
    .bind(first_start_ms)
    .fetch_all(pool)
    .await?;
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT invocation_row_id, bucket_kind, bucket_date, bucket_key, dimension, series_key, interval_start_ms, interval_end_ms FROM long_term_projection_intervals legacy WHERE bucket_date IN (",
    );
    let mut separated = builder.separated(", ");
    for date in dates {
        separated.push_bind(date);
    }
    separated.push_unseparated(") AND NOT EXISTS (SELECT 1 FROM long_term_projection_interval_state state WHERE state.invocation_row_id = legacy.invocation_row_id)");
    let rows = builder
        .build_query_as::<LongTermProjectionLegacyIntervalRow>()
        .fetch_all(pool)
        .await?;
    let mut index = HashMap::new();
    for state in canonical_rows {
        for (date_text, date) in &requested_dates {
            if !suppressed.contains(&(state.invocation_row_id, (*date_text).to_string())) {
                add_long_term_projection_interval_state_for_date(&mut index, *date, &state);
            }
        }
    }
    for row in rows {
        if suppressed.contains(&(row.invocation_row_id, row.bucket_date.clone())) {
            continue;
        }
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

fn add_long_term_projection_interval_state_for_date(
    index: &mut HashMap<LongTermProjectionIntervalKey, LongTermProjectionIntervalUnion>,
    date: NaiveDate,
    state: &LongTermProjectionIntervalStateRow,
) {
    let Some((day_start_epoch, day_end_epoch)) = long_term_day_epoch_bounds(date) else {
        return;
    };
    let day_start_ms = day_start_epoch * 1_000;
    let day_end_ms = day_end_epoch * 1_000;
    let clipped_start_ms = state.interval_start_ms.max(day_start_ms);
    let clipped_end_ms = state.interval_end_ms.min(day_end_ms);
    if clipped_end_ms <= clipped_start_ms {
        return;
    }
    for (dimension, series_key) in [
        ("overall", "overall"),
        ("model", state.model_series_key.as_str()),
        ("upstream", state.upstream_series_key.as_str()),
    ] {
        index
            .entry(projection_interval_key(
                "daily",
                date.to_string(),
                dimension.to_string(),
                series_key.to_string(),
            ))
            .or_default()
            .add(clipped_start_ms, clipped_end_ms);
    }
    let mut hour_start_ms = clipped_start_ms.div_euclid(LONG_TERM_HOUR_MS) * LONG_TERM_HOUR_MS;
    while hour_start_ms < clipped_end_ms {
        let hour_end_ms = hour_start_ms.saturating_add(LONG_TERM_HOUR_MS);
        let hour_clipped_start_ms = clipped_start_ms.max(hour_start_ms);
        let hour_clipped_end_ms = clipped_end_ms.min(hour_end_ms);
        for (dimension, series_key) in [
            ("overall", "overall"),
            ("model", state.model_series_key.as_str()),
            ("upstream", state.upstream_series_key.as_str()),
        ] {
            index
                .entry(projection_interval_key(
                    "hourly",
                    (hour_start_ms / 1_000).to_string(),
                    dimension.to_string(),
                    series_key.to_string(),
                ))
                .or_default()
                .add(hour_clipped_start_ms, hour_clipped_end_ms);
        }
        hour_start_ms = hour_end_ms;
    }
}

fn long_term_projection_interval_dates(
    segment: &LongTermProjectionIntervalSegment,
) -> HashSet<String> {
    let mut dates = HashSet::new();
    let Some(mut date) = Shanghai
        .timestamp_millis_opt(segment.interval_start_ms)
        .single()
        .map(|timestamp| timestamp.date_naive())
    else {
        return dates;
    };
    while let Some((_, day_end_epoch)) = long_term_day_epoch_bounds(date) {
        dates.insert(date.to_string());
        if day_end_epoch.saturating_mul(1_000) >= segment.interval_end_ms {
            break;
        }
        let Some(next) = date.succ_opt() else {
            break;
        };
        date = next;
    }
    dates
}

async fn upsert_long_term_projection_interval_segments(
    pool: &Pool<Sqlite>,
    segments: &[LongTermProjectionIntervalSegment],
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    for batch in segments.chunks(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS) {
        let (mut transaction, permit) = control.begin(pool).await?;
        upsert_long_term_projection_interval_segments_in_transaction(&mut transaction, batch)
            .await?;
        control.commit(transaction, permit).await?;
    }
    Ok(())
}

async fn upsert_long_term_projection_interval_segments_in_transaction(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    segments: &[LongTermProjectionIntervalSegment],
) -> Result<()> {
    for segment in segments {
        sqlx::query(
            "INSERT INTO long_term_projection_interval_state (invocation_row_id, model_series_key, upstream_series_key, interval_start_ms, interval_end_ms) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(invocation_row_id) DO UPDATE SET model_series_key = excluded.model_series_key, upstream_series_key = excluded.upstream_series_key, interval_start_ms = excluded.interval_start_ms, interval_end_ms = excluded.interval_end_ms",
        )
        .bind(segment.invocation_row_id)
        .bind(&segment.model_series_key)
        .bind(&segment.upstream_series_key)
        .bind(segment.interval_start_ms)
        .bind(segment.interval_end_ms)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

async fn load_long_term_projection_interval_state_ids(
    pool: &Pool<Sqlite>,
    segments: &[LongTermProjectionIntervalSegment],
) -> Result<HashSet<i64>> {
    if segments.is_empty() {
        return Ok(HashSet::new());
    }
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT invocation_row_id FROM long_term_projection_interval_state WHERE invocation_row_id IN (",
    );
    let mut ids = builder.separated(", ");
    for segment in segments {
        ids.push_bind(segment.invocation_row_id);
    }
    ids.push_unseparated(")");
    Ok(builder
        .build_query_scalar::<i64>()
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect())
}

async fn migrate_long_term_projection_legacy_interval_state(
    pool: &Pool<Sqlite>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<bool> {
    let legacy_rows = sqlx::query_as::<_, LongTermProjectionLegacyCompactRow>(
        r#"
        SELECT legacy.invocation_row_id,
               MAX(CASE WHEN legacy.dimension = 'model' THEN legacy.series_key END) AS model_series_key,
               MAX(CASE WHEN legacy.dimension = 'upstream' THEN legacy.series_key END) AS upstream_series_key,
               MIN(legacy.interval_start_ms) AS interval_start_ms,
               MAX(legacy.interval_end_ms) AS interval_end_ms
        FROM long_term_projection_intervals legacy
        WHERE NOT EXISTS (
            SELECT 1
            FROM long_term_projection_interval_state state
            WHERE state.invocation_row_id = legacy.invocation_row_id
        )
        GROUP BY legacy.invocation_row_id
        ORDER BY legacy.invocation_row_id ASC
        LIMIT ?1
        "#,
    )
    .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
    .fetch_all(pool)
    .await?;
    if !legacy_rows.is_empty() {
        let segments = legacy_rows
            .into_iter()
            .filter(|row| row.interval_end_ms > row.interval_start_ms)
            .map(|row| LongTermProjectionIntervalSegment {
                invocation_row_id: row.invocation_row_id,
                model_series_key: row
                    .model_series_key
                    .unwrap_or_else(|| LONG_TERM_OTHER_KEY.to_string()),
                upstream_series_key: row
                    .upstream_series_key
                    .unwrap_or_else(|| LONG_TERM_OTHER_KEY.to_string()),
                interval_start_ms: row.interval_start_ms,
                interval_end_ms: row.interval_end_ms,
            })
            .collect::<Vec<_>>();
        if segments.is_empty() {
            return Ok(false);
        }
        upsert_long_term_projection_interval_segments(pool, &segments, control).await?;
        return Ok(true);
    }

    let cleanup_pending = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM long_term_projection_intervals legacy WHERE EXISTS (SELECT 1 FROM long_term_projection_interval_state state WHERE state.invocation_row_id = legacy.invocation_row_id) LIMIT 1)",
    )
    .fetch_one(pool)
    .await?
        != 0;
    if !cleanup_pending {
        return Ok(false);
    }
    let (mut transaction, permit) = control.begin(pool).await?;
    let deleted = sqlx::query(
        "DELETE FROM long_term_projection_intervals WHERE rowid IN (SELECT legacy.rowid FROM long_term_projection_intervals legacy WHERE EXISTS (SELECT 1 FROM long_term_projection_interval_state state WHERE state.invocation_row_id = legacy.invocation_row_id) LIMIT ?1)",
    )
    .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    control.commit(transaction, permit).await?;
    Ok(deleted != 0)
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
    let control = LongTermProjectionWriteControl::background(
        &state.shutdown,
        crate::db_pressure::global_db_pressure_gate(),
    );
    let _ = apply_long_term_projection_incremental_with_runtime_and_control(
        &state.pool,
        &state.long_term_projection_runtime,
        LongTermProjectionIncrementalBatch {
            hourly,
            daily,
            segments,
        },
        next_cursor,
        event_count,
        &control,
    )
    .await?;
    Ok(())
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
    let control = LongTermProjectionWriteControl::unrestricted();
    let _ = apply_long_term_projection_incremental_with_runtime_and_control(
        pool,
        runtime,
        LongTermProjectionIncrementalBatch {
            hourly,
            daily,
            segments,
        },
        next_cursor,
        event_count,
        &control,
    )
    .await?;
    Ok(())
}

struct LongTermProjectionIncrementalBatch<'a> {
    hourly: &'a HashMap<(i64, String, String), LongTermBucket>,
    daily: &'a HashMap<(String, String, String), LongTermBucket>,
    segments: &'a [LongTermProjectionIntervalSegment],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LongTermProjectionIncrementalOutcome {
    Published,
    RebuildRequired,
}

async fn long_term_projection_incremental_dates_are_dirty(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    dates: &HashSet<String>,
) -> Result<bool> {
    if dates.is_empty() {
        return Ok(false);
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT EXISTS(SELECT 1 FROM long_term_projection_dirty_buckets WHERE bucket_date IN (",
    );
    {
        let mut separated = query.separated(", ");
        for date in dates {
            separated.push_bind(date);
        }
    }
    query.push("))");
    Ok(query
        .build_query_scalar::<i64>()
        .fetch_one(&mut **transaction)
        .await?
        != 0)
}

async fn apply_long_term_projection_incremental_with_runtime_and_control(
    pool: &Pool<Sqlite>,
    runtime: &Arc<Mutex<LongTermProjectionRuntime>>,
    batch: LongTermProjectionIncrementalBatch<'_>,
    next_cursor: i64,
    event_count: usize,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<LongTermProjectionIncrementalOutcome> {
    let rollup_rows = batch.hourly.len() + batch.daily.len();
    let mutation_rows = rollup_rows + batch.segments.len();
    if mutation_rows > LONG_TERM_PROJECTION_INCREMENTAL_MUTATION_ROWS {
        bail!(
            "long-term projection incremental batch has {mutation_rows} writes; maximum is {LONG_TERM_PROJECTION_INCREMENTAL_MUTATION_ROWS}"
        );
    }
    let mut dates = HashSet::new();
    for segment in batch.segments {
        dates.extend(long_term_projection_interval_dates(segment));
    }
    for bucket in batch.daily.values() {
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
    let persisted_segment_ids =
        load_long_term_projection_interval_state_ids(pool, batch.segments).await?;
    for segment in batch
        .segments
        .iter()
        .filter(|segment| !persisted_segment_ids.contains(&segment.invocation_row_id))
    {
        let state = LongTermProjectionIntervalStateRow {
            invocation_row_id: segment.invocation_row_id,
            model_series_key: segment.model_series_key.clone(),
            upstream_series_key: segment.upstream_series_key.clone(),
            interval_start_ms: segment.interval_start_ms,
            interval_end_ms: segment.interval_end_ms,
        };
        for date in &dates {
            if let Ok(date) = NaiveDate::parse_from_str(date, "%Y-%m-%d") {
                add_long_term_projection_interval_state_for_date(&mut interval_index, date, &state);
            }
        }
    }
    let (mut tx, permit) = control.begin(pool).await?;
    if long_term_projection_incremental_dates_are_dirty(&mut tx, &dates).await? {
        drop(tx);
        drop(permit);
        return Ok(LongTermProjectionIncrementalOutcome::RebuildRequired);
    }
    upsert_long_term_projection_interval_segments_in_transaction(&mut tx, batch.segments).await?;
    for bucket in batch.hourly.values() {
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
    for bucket in batch.daily.values() {
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
    let statistics_start_date = batch
        .daily
        .values()
        .filter_map(|bucket| bucket.stats_date.as_deref())
        .min()
        .map(str::to_string);
    sqlx::query(
        "UPDATE long_term_stats_state SET status = CASE WHEN ?1 > 0 AND status <> ?2 AND NOT EXISTS (SELECT 1 FROM long_term_projection_dirty_buckets) THEN ?3 ELSE status END, statistics_start_date = CASE WHEN ?4 IS NULL THEN statistics_start_date WHEN statistics_start_date IS NULL OR ?4 < statistics_start_date THEN ?4 ELSE statistics_start_date END, processed_rows = processed_rows + ?1, total_rows = total_rows + ?1, last_error = CASE WHEN ?1 > 0 AND status <> ?2 AND NOT EXISTS (SELECT 1 FROM long_term_projection_dirty_buckets) THEN NULL ELSE last_error END, updated_at = datetime('now') WHERE id = ?5",
    )
    .bind(event_count as i64)
    .bind(LONG_TERM_STATUS_ERROR)
    .bind(LONG_TERM_STATUS_READY)
    .bind(statistics_start_date)
    .bind(LONG_TERM_STATE_ID)
    .execute(&mut *tx)
    .await?;
    control.commit(tx, permit).await?;
    let mut runtime = runtime.lock().await;
    runtime.interval_index = interval_index;
    runtime.loaded_interval_dates = loaded_dates;
    Ok(LongTermProjectionIncrementalOutcome::Published)
}

async fn ensure_long_term_projection_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_state (
            consumer TEXT PRIMARY KEY,
            cursor_row_id INTEGER NOT NULL DEFAULT 0,
            last_flush_at TEXT,
            last_daily_verify_at TEXT,
            daily_verify_pending INTEGER NOT NULL DEFAULT 0,
            daily_verify_bucket_date TEXT,
            last_error TEXT,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection state table")?;
    let daily_verify_migration =
        sqlx::query("ALTER TABLE long_term_projection_state ADD COLUMN last_daily_verify_at TEXT")
            .execute(pool)
            .await;
    if let Err(error) = daily_verify_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    let daily_verify_pending_migration = sqlx::query(
        "ALTER TABLE long_term_projection_state ADD COLUMN daily_verify_pending INTEGER NOT NULL DEFAULT 0",
    )
    .execute(pool)
    .await;
    if let Err(error) = daily_verify_pending_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    let daily_verify_bucket_date_migration = sqlx::query(
        "ALTER TABLE long_term_projection_state ADD COLUMN daily_verify_bucket_date TEXT",
    )
    .execute(pool)
    .await;
    if let Err(error) = daily_verify_bucket_date_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_dirty_buckets (
            bucket_date TEXT PRIMARY KEY,
            repair_reason TEXT NOT NULL,
            generation INTEGER NOT NULL DEFAULT 1,
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
    let generation_migration = sqlx::query(
        "ALTER TABLE long_term_projection_dirty_buckets ADD COLUMN generation INTEGER NOT NULL DEFAULT 1",
    )
    .execute(pool)
    .await;
    if let Err(error) = generation_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_bucket_state (
            bucket_date TEXT PRIMARY KEY,
            interval_baseline_ready INTEGER NOT NULL DEFAULT 0,
            active_daily_backup_token TEXT,
            publication_token TEXT,
            publication_generation INTEGER,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection bucket state table")?;
    let daily_backup_token_migration = sqlx::query(
        "ALTER TABLE long_term_projection_bucket_state ADD COLUMN active_daily_backup_token TEXT",
    )
    .execute(pool)
    .await;
    if let Err(error) = daily_backup_token_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    let publication_token_migration = sqlx::query(
        "ALTER TABLE long_term_projection_bucket_state ADD COLUMN publication_token TEXT",
    )
    .execute(pool)
    .await;
    if let Err(error) = publication_token_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    let publication_generation_migration = sqlx::query(
        "ALTER TABLE long_term_projection_bucket_state ADD COLUMN publication_generation INTEGER",
    )
    .execute(pool)
    .await;
    if let Err(error) = publication_generation_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_daily_backups (
            rebuild_token TEXT NOT NULL,
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
            PRIMARY KEY (rebuild_token, stats_date, dimension, series_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection daily backup table")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_date_publications (
            publication_token TEXT PRIMARY KEY,
            published INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection date publication table")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_archive_compatibility (
            file_path TEXT PRIMARY KEY,
            archive_sha256 TEXT NOT NULL,
            file_fingerprint TEXT,
            has_legacy_crossing INTEGER NOT NULL,
            legacy_max_duration_ms REAL,
            legacy_min_occurred_at TEXT,
            has_rfc3339 INTEGER NOT NULL,
            rfc3339_max_duration_ms REAL,
            rfc3339_min_occurred_at TEXT,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection archive compatibility table")?;
    let archive_legacy_duration_migration = sqlx::query(
        "ALTER TABLE long_term_projection_archive_compatibility ADD COLUMN legacy_max_duration_ms REAL",
    )
    .execute(pool)
    .await;
    if let Err(error) = archive_legacy_duration_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    let archive_legacy_start_migration = sqlx::query(
        "ALTER TABLE long_term_projection_archive_compatibility ADD COLUMN legacy_min_occurred_at TEXT",
    )
    .execute(pool)
    .await;
    if let Err(error) = archive_legacy_start_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    let archive_fingerprint_migration = sqlx::query(
        "ALTER TABLE long_term_projection_archive_compatibility ADD COLUMN file_fingerprint TEXT",
    )
    .execute(pool)
    .await;
    if let Err(error) = archive_fingerprint_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    let archive_rfc3339_duration_migration = sqlx::query(
        "ALTER TABLE long_term_projection_archive_compatibility ADD COLUMN rfc3339_max_duration_ms REAL",
    )
    .execute(pool)
    .await;
    if let Err(error) = archive_rfc3339_duration_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    let archive_rfc3339_start_migration = sqlx::query(
        "ALTER TABLE long_term_projection_archive_compatibility ADD COLUMN rfc3339_min_occurred_at TEXT",
    )
    .execute(pool)
    .await;
    if let Err(error) = archive_rfc3339_start_migration
        && !error.to_string().contains("duplicate column name")
    {
        return Err(error.into());
    }
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_long_term_projection_daily_backups_token_date ON long_term_projection_daily_backups (rebuild_token, stats_date)",
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection daily backup index")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_daily_backup_claims (
            bucket_date TEXT PRIMARY KEY,
            rebuild_token TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection daily backup claim table")?;
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
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_long_term_projection_intervals_invocation ON long_term_projection_intervals (invocation_row_id)",
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection interval invocation index")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_interval_state (
            invocation_row_id INTEGER PRIMARY KEY,
            model_series_key TEXT NOT NULL,
            upstream_series_key TEXT NOT NULL,
            interval_start_ms INTEGER NOT NULL,
            interval_end_ms INTEGER NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection canonical interval state table")?;
    sqlx::query("DROP INDEX IF EXISTS idx_long_term_projection_interval_state_range")
        .execute(pool)
        .await
        .context("failed to replace long-term projection interval range index")?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_long_term_projection_interval_state_end_start ON long_term_projection_interval_state (interval_end_ms, interval_start_ms)",
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection canonical interval range index")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_interval_suppressions (
            invocation_row_id INTEGER NOT NULL,
            bucket_date TEXT NOT NULL,
            PRIMARY KEY (invocation_row_id, bucket_date)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection interval suppression table")?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_long_term_projection_interval_suppressions_date ON long_term_projection_interval_suppressions (bucket_date, invocation_row_id)",
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection interval suppression date index")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_rebuild_members (
            rebuild_token TEXT NOT NULL,
            invocation_row_id INTEGER NOT NULL,
            PRIMARY KEY (rebuild_token, invocation_row_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long-term projection rebuild membership table")?;
    Ok(())
}

async fn load_long_term_rollups(
    pool: &Pool<Sqlite>,
    range: LongTermRange,
    dimension: &str,
    start_date: &str,
    end_date: &str,
) -> Result<Vec<LongTermRollupRow>> {
    let table = "active_daily";
    let sql = format!(
        "{} SELECT MAX(d.stats_date) AS bucket_or_date, d.dimension, d.series_key, COALESCE((SELECT account.display_name FROM pool_upstream_accounts account WHERE d.dimension = 'upstream' AND d.series_key LIKE 'account:%' AND account.id = CAST(substr(d.series_key, 9) AS INTEGER) LIMIT 1), (SELECT latest.display_name FROM {table} latest WHERE latest.dimension = ?1 AND latest.series_key = d.series_key AND latest.reasoning_effort = d.reasoning_effort AND latest.stats_date BETWEEN ?2 AND ?3 ORDER BY latest.stats_date DESC LIMIT 1)) AS display_name, d.reasoning_effort, SUM(d.calls) AS calls, SUM(d.token_total) AS token_total, SUM(d.token_samples) AS token_samples, SUM(d.cost_total) AS cost_total, SUM(d.cost_samples) AS cost_samples, SUM(d.usage_time_ms) AS usage_time_ms, SUM(d.usage_time_samples) AS usage_time_samples, SUM(d.wall_time_ms) AS wall_time_ms, SUM(d.wall_time_samples) AS wall_time_samples, SUM(d.output_tokens_total) AS output_tokens_total, SUM(d.stream_duration_ms) AS stream_duration_ms, SUM(d.output_speed_samples) AS output_speed_samples, SUM(d.first_byte_sum_ms) AS first_byte_sum_ms, SUM(d.first_byte_samples) AS first_byte_samples, SUM(d.response_sum_ms) AS response_sum_ms, SUM(d.response_samples) AS response_samples FROM {table} d WHERE d.dimension = ?1 AND d.stats_date BETWEEN ?2 AND ?3 GROUP BY d.series_key, d.reasoning_effort",
        long_term_projection_active_daily_cte(),
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
    let mut sql = format!(
        "{} SELECT stats_date AS bucket_or_date, dimension, series_key, display_name, reasoning_effort, calls, token_total, token_samples, cost_total, cost_samples, usage_time_ms, usage_time_samples, wall_time_ms, wall_time_samples, output_tokens_total, stream_duration_ms, output_speed_samples, first_byte_sum_ms, first_byte_samples, response_sum_ms, response_samples FROM active_daily WHERE dimension = ?1 AND stats_date BETWEEN ?2 AND ?3",
        long_term_projection_active_daily_cte(),
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

fn long_term_projection_active_daily_cte() -> &'static str {
    r#"
    WITH active_daily AS (
      SELECT d.stats_date, d.dimension, d.series_key, d.display_name, d.reasoning_effort,
        d.calls, d.token_total, d.token_samples, d.cost_total, d.cost_samples,
        d.usage_time_ms, d.usage_time_samples, d.wall_time_ms, d.wall_time_samples,
        d.output_tokens_total, d.stream_duration_ms, d.output_speed_samples,
        d.first_byte_sum_ms, d.first_byte_samples, d.response_sum_ms, d.response_samples
      FROM long_term_usage_daily d
      LEFT JOIN long_term_projection_bucket_state state ON state.bucket_date = d.stats_date
      LEFT JOIN long_term_projection_date_publications publication
        ON publication.publication_token = state.publication_token
      WHERE state.active_daily_backup_token IS NULL
         OR (
           publication.published = 1
           AND NOT EXISTS (
             SELECT 1
             FROM long_term_projection_dirty_buckets dirty
             WHERE dirty.bucket_date = d.stats_date
               AND (
                 state.publication_generation IS NULL
                 OR dirty.generation <> state.publication_generation
               )
           )
         )
      UNION ALL
      SELECT backup.stats_date, backup.dimension, backup.series_key, backup.display_name,
        backup.reasoning_effort, backup.calls, backup.token_total, backup.token_samples,
        backup.cost_total, backup.cost_samples, backup.usage_time_ms,
        backup.usage_time_samples, backup.wall_time_ms, backup.wall_time_samples,
        backup.output_tokens_total, backup.stream_duration_ms, backup.output_speed_samples,
        backup.first_byte_sum_ms, backup.first_byte_samples, backup.response_sum_ms,
        backup.response_samples
      FROM long_term_projection_daily_backups backup
      JOIN long_term_projection_bucket_state state
        ON state.bucket_date = backup.stats_date
       AND state.active_daily_backup_token = backup.rebuild_token
      LEFT JOIN long_term_projection_date_publications publication
        ON publication.publication_token = state.publication_token
      WHERE publication.published IS NULL
         OR publication.published = 0
         OR EXISTS (
           SELECT 1
           FROM long_term_projection_dirty_buckets dirty
           WHERE dirty.bucket_date = backup.stats_date
             AND (
               state.publication_generation IS NULL
               OR dirty.generation <> state.publication_generation
             )
         )
    )
    "#
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

#[derive(Debug, Clone, Copy)]
struct LongTermTimestamp {
    epoch_ms: i64,
    sub_millisecond_nanos: u32,
}

fn parse_long_term_timestamp(raw: &str) -> Option<LongTermTimestamp> {
    if let Ok(value) = DateTime::parse_from_rfc3339(raw) {
        return Some(LongTermTimestamp {
            epoch_ms: value.timestamp_millis(),
            sub_millisecond_nanos: value.timestamp_subsec_nanos() % 1_000_000,
        });
    }
    let naive = NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f")
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S"))
        .ok()?;
    Shanghai
        .from_local_datetime(&naive)
        .single()
        .map(|value| LongTermTimestamp {
            epoch_ms: value.timestamp_millis(),
            sub_millisecond_nanos: value.timestamp_subsec_nanos() % 1_000_000,
        })
}

fn parse_long_term_timestamp_ms(raw: &str) -> Option<i64> {
    parse_long_term_timestamp(raw).map(|timestamp| timestamp.epoch_ms)
}

fn long_term_interval_end_ms(start: LongTermTimestamp, duration_ms: f64) -> Option<i64> {
    let elapsed_nanos = duration_ms * 1_000_000.0;
    if !elapsed_nanos.is_finite() || elapsed_nanos <= 0.0 {
        return None;
    }
    // Interval state is stored in milliseconds. Round its exclusive end up so a positive
    // sub-millisecond tail that crosses a date or hour boundary remains materialized there.
    let elapsed_millis = ((start.sub_millisecond_nanos as f64 + elapsed_nanos) / 1_000_000.0)
        .ceil()
        .clamp(1.0, i64::MAX as f64) as i64;
    start.epoch_ms.checked_add(elapsed_millis)
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

    #[test]
    fn terminal_and_maintenance_deadlines_defer_expensive_repairs() {
        assert!(!long_term_projection_allows_expensive_repair(
            "terminal_deadline"
        ));
        assert!(long_term_projection_allows_expensive_repair(
            "repair_deadline"
        ));
        assert!(long_term_projection_allows_expensive_repair("daily_verify"));
        assert!(!long_term_projection_allows_expensive_repair(
            "maintenance_deadline"
        ));
    }

    #[test]
    fn initial_materializer_only_runs_before_a_durable_long_term_baseline() {
        assert!(long_term_initial_materialization_needed(
            LONG_TERM_STATUS_PREPARING,
            None,
            false,
        ));
        assert!(long_term_initial_materialization_needed(
            LONG_TERM_STATUS_RUNNING,
            None,
            false,
        ));
        assert!(!long_term_initial_materialization_needed(
            LONG_TERM_STATUS_READY,
            None,
            false,
        ));
        assert!(!long_term_initial_materialization_needed(
            LONG_TERM_STATUS_PREPARING,
            None,
            true,
        ));
        assert!(long_term_initial_materialization_needed(
            LONG_TERM_STATUS_ERROR,
            None,
            true,
        ));
        assert!(long_term_initial_materialization_needed(
            LONG_TERM_STATUS_RUNNING,
            Some(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR),
            true,
        ));
    }

    #[test]
    fn archive_scan_identity_requires_stable_bytes_and_matching_manifest() {
        assert!(long_term_archive_scan_identity_matches_manifest(
            "scanned-sha",
            Some("scanned-sha"),
            Some("scanned-sha"),
        ));
        assert!(!long_term_archive_scan_identity_matches_manifest(
            "scanned-sha",
            Some("rewritten-sha"),
            Some("scanned-sha"),
        ));
        assert!(!long_term_archive_scan_identity_matches_manifest(
            "scanned-sha",
            Some("scanned-sha"),
            Some("stale-manifest-sha"),
        ));
    }

    #[cfg(unix)]
    #[test]
    fn archive_file_identity_changes_when_a_prepared_replacement_is_renamed() {
        let archive_path = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-replay-identity-{}-{}.sqlite.gz",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let replacement_path = archive_path.with_extension("replacement");
        fs::write(&archive_path, b"scanned archive bytes").expect("write scanned archive");
        let scanned_identity =
            long_term_archive_file_identity(archive_path.to_string_lossy().as_ref())
                .expect("read scanned archive identity");
        fs::write(&replacement_path, b"prepared replacement archive bytes")
            .expect("write prepared replacement archive");
        fs::rename(&replacement_path, &archive_path).expect("rename prepared replacement");
        let replacement_identity =
            long_term_archive_file_identity(archive_path.to_string_lossy().as_ref())
                .expect("read replacement archive identity");

        assert_ne!(scanned_identity, replacement_identity);
        let _ = fs::remove_file(&archive_path);
    }

    #[tokio::test]
    async fn archive_source_identity_rejects_a_valid_file_with_a_stale_manifest() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("full schema");
        let archive_path = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-stale-archive-manifest-{}-{}.sqlite.gz",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::write(&archive_path, b"valid replacement archive bytes")
            .expect("write replacement archive");
        let archive_path_text = archive_path.to_string_lossy().to_string();
        let archive_sha256 =
            crate::maintenance::sha256_hex_file(&archive_path).expect("hash replacement archive");
        sqlx::query(
            "INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, created_at) VALUES ('pool_upstream_request_attempts', '2026-08', ?1, 'stale-sha', 1, 'completed', datetime('now'))",
        )
        .bind(&archive_path_text)
        .execute(&pool)
        .await
        .expect("record stale manifest");

        let error = ensure_long_term_archive_source_identity(
            &pool,
            "pool_upstream_request_attempts",
            &archive_path_text,
            "stale-sha",
        )
        .await
        .expect_err("a valid replacement must not satisfy its stale manifest");
        assert!(error.to_string().contains("does not match"));

        sqlx::query("UPDATE archive_batches SET sha256 = ?1 WHERE file_path = ?2")
            .bind(&archive_sha256)
            .bind(&archive_path_text)
            .execute(&pool)
            .await
            .expect("update manifest to replacement identity");
        ensure_long_term_archive_source_identity(
            &pool,
            "pool_upstream_request_attempts",
            &archive_path_text,
            &archive_sha256,
        )
        .await
        .expect("matching manifest accepts the source");
        let _ = fs::remove_file(&archive_path);
    }

    #[test]
    fn long_term_projection_repair_retries_deferred_and_persisted_dirty_work() {
        let now = Instant::now();

        assert!(long_term_projection_repair_due(
            false, true, false, None, now,
        ));
        assert!(long_term_projection_repair_due(
            false,
            false,
            false,
            Some(now - Duration::from_millis(1)),
            now,
        ));
        assert!(!long_term_projection_repair_due(
            false,
            false,
            false,
            Some(now + Duration::from_secs(1)),
            now,
        ));
    }

    #[test]
    fn long_term_projection_repair_respects_a_future_deadline_with_pending_work() {
        let now = Instant::now();
        assert!(!long_term_projection_repair_due(
            true,
            true,
            true,
            Some(now + Duration::from_secs(1)),
            now,
        ));
    }

    #[tokio::test]
    async fn daily_verify_pressure_rejection_stays_due_and_wakes_after_gate_release() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        sqlx::query(
            "INSERT INTO long_term_projection_state (consumer, cursor_row_id) VALUES (?1, 0)",
        )
        .bind(LONG_TERM_PROJECTION_CONSUMER)
        .execute(&pool)
        .await
        .expect("seed projection state");
        assert!(
            long_term_projection_daily_verify_due(&pool)
                .await
                .expect("missing verification is due")
        );
        let shutdown = CancellationToken::new();
        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
        let held = gate
            .try_begin_background("test-daily-verify-pressure")
            .expect("hold background admission");
        let observed_generation = gate.eligibility_generation();
        let control = LongTermProjectionWriteControl::background(&shutdown, &gate);
        let error = queue_long_term_projection_daily_verify_with_control(
            &pool,
            &Utc::now().with_timezone(&Shanghai).date_naive().to_string(),
            &control,
        )
        .await
        .expect_err("pressure must reject daily verification persistence");
        assert!(long_term_projection_write_is_deferred(&error));
        assert!(
            long_term_projection_daily_verify_due(&pool)
                .await
                .expect("pressure rejection keeps daily verification due")
        );
        let wait_for_release = wait_for_long_term_projection_pressure_retry(
            &gate,
            observed_generation,
            long_term_projection_pressure_retry_at(&gate),
        );
        drop(held);
        tokio::time::timeout(Duration::from_millis(100), wait_for_release)
            .await
            .expect("daily verification retry wakes as soon as the background slot releases");
        assert!(
            long_term_projection_daily_verify_due(&pool)
                .await
                .expect("the persisted verification obligation survives the pressure retry")
        );
        sqlx::query(
            "UPDATE long_term_projection_state SET last_daily_verify_at = datetime('now') WHERE consumer = ?1",
        )
        .bind(LONG_TERM_PROJECTION_CONSUMER)
        .execute(&pool)
        .await
        .expect("persist verification success");
        assert!(
            !long_term_projection_daily_verify_due(&pool)
                .await
                .expect("fresh verification is not due")
        );
    }

    #[tokio::test]
    async fn initial_materialization_control_rejects_progress_and_integrity_writes() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        let mismatch = LongTermIntegrityMismatch {
            date: NaiveDate::from_ymd_opt(2026, 8, 20).expect("repair date"),
            expected: LongTermIntegrityTotals {
                calls: 2,
                ..LongTermIntegrityTotals::default()
            },
            observed: LongTermIntegrityTotals {
                calls: 1,
                ..LongTermIntegrityTotals::default()
            },
            reason: "controlled write test".to_string(),
        };
        let shutdown = CancellationToken::new();
        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
        let held = gate
            .try_begin_background("test-initial-materialization-pressure")
            .expect("hold background admission");
        let control = LongTermProjectionWriteControl::background(&shutdown, &gate);

        for error in [
            persist_long_term_refresh_progress(&pool, 256, 512, &control)
                .await
                .expect_err("pressure rejects progress write"),
            enqueue_long_term_integrity_mismatch(&pool, &mismatch, &control)
                .await
                .expect_err("pressure rejects repair queue write"),
            mark_long_term_integrity_audit(&pool, &control)
                .await
                .expect_err("pressure rejects audit marker write"),
        ] {
            assert!(long_term_projection_write_is_deferred(&error));
        }
        assert_eq!(
            sqlx::query_as::<_, (i64, i64)>(
                "SELECT processed_rows, total_rows FROM long_term_stats_state WHERE id = ?1",
            )
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("unchanged progress"),
            (0, 0)
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_stats_repair_queue")
                .fetch_one(&pool)
                .await
                .expect("empty repair queue"),
            0
        );
        assert!(
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT last_integrity_audit_at FROM long_term_stats_state WHERE id = ?1",
            )
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("missing audit marker")
            .is_none()
        );

        drop(held);
        shutdown.cancel();
        for error in [
            persist_long_term_refresh_progress(&pool, 256, 512, &control)
                .await
                .expect_err("shutdown rejects progress write"),
            enqueue_long_term_integrity_mismatch(&pool, &mismatch, &control)
                .await
                .expect_err("shutdown rejects repair queue write"),
            mark_long_term_integrity_audit(&pool, &control)
                .await
                .expect_err("shutdown rejects audit marker write"),
        ] {
            assert!(long_term_projection_write_is_deferred(&error));
        }
    }

    #[tokio::test]
    async fn initial_materialization_control_waits_for_a_short_lived_background_slot() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        let shutdown = CancellationToken::new();
        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
        let held = gate
            .try_begin_background("test-short-lived-background-slot")
            .expect("hold background admission");
        let control = LongTermProjectionWriteControl::background(&shutdown, &gate);

        let release = async move {
            sleep(Duration::from_millis(10)).await;
            drop(held);
        };
        let write = persist_long_term_refresh_progress(&pool, 256, 512, &control);
        let (_, result) = tokio::join!(release, write);
        result.expect("a short-lived background slot is admitted within the P2 window");
        assert_eq!(
            sqlx::query_as::<_, (i64, i64)>(
                "SELECT processed_rows, total_rows FROM long_term_stats_state WHERE id = ?1",
            )
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("persisted progress"),
            (256, 512)
        );
    }

    #[tokio::test]
    async fn controlled_transaction_cancels_while_an_external_writer_holds_sqlite() {
        let (pool, _db_url, db_path) = long_term_file_backed_pool_with_busy_timeout(
            "cancel-locked-write",
            Duration::from_secs(30),
        )
        .await;
        let external_writer = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .expect("hold the external SQLite writer lock");
        let shutdown = CancellationToken::new();
        let cancellation = shutdown.clone();
        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
        let control = LongTermProjectionWriteControl::background(&shutdown, &gate);

        let cancel = async move {
            sleep(Duration::from_millis(20)).await;
            cancellation.cancel();
        };
        let blocked_begin = tokio::time::timeout(Duration::from_millis(200), control.begin(&pool));
        let (_, result) = tokio::join!(cancel, blocked_begin);
        let error = result
            .expect("shutdown cancels the blocked transaction before SQLite's busy timeout")
            .expect_err("locked transaction admission is cancelled");
        assert!(long_term_projection_write_is_deferred(&error));
        let permit = gate
            .try_begin_background("verify-cancelled-transaction-released-permit")
            .expect("cancellation releases the P2 admission permit");
        drop(permit);
        external_writer
            .rollback()
            .await
            .expect("release external SQLite writer lock");
        cleanup_long_term_file_backed_pool(pool, db_path).await;
    }

    #[tokio::test]
    async fn controlled_transaction_timeout_waits_for_pressure_cooldown_before_retry() {
        let (pool, _db_url, db_path) = long_term_file_backed_pool_with_busy_timeout(
            "cooldown-locked-write",
            Duration::from_secs(30),
        )
        .await;
        let external_writer = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .expect("hold the external SQLite writer lock");
        let shutdown = CancellationToken::new();
        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_millis(50));
        let control = LongTermProjectionWriteControl::background(&shutdown, &gate);
        let observed_generation = gate.eligibility_generation();

        let error = tokio::time::timeout(Duration::from_millis(400), control.begin(&pool))
            .await
            .expect("bounded transaction admission")
            .expect_err("external writer keeps transaction admission blocked");
        assert!(long_term_projection_write_is_pressure_deferred(&error));
        let retry_at = long_term_projection_pressure_retry_at(&gate)
            .expect("transaction admission timeout starts a pressure cooldown");
        assert!(matches!(
            gate.background_deny_reason(),
            Some(crate::db_pressure::DbPressureDenyReason::PressureCooldown { .. })
        ));
        assert!(
            tokio::time::timeout(
                Duration::from_millis(10),
                wait_for_long_term_projection_pressure_retry(
                    &gate,
                    observed_generation,
                    Some(retry_at)
                ),
            )
            .await
            .is_err()
        );

        external_writer
            .rollback()
            .await
            .expect("release external SQLite writer lock");
        cleanup_long_term_file_backed_pool(pool, db_path).await;
    }

    #[tokio::test]
    async fn daily_verify_stays_due_until_its_queued_bucket_is_rebuilt() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        sqlx::query(
            "INSERT INTO long_term_projection_state (consumer, cursor_row_id) VALUES (?1, 0)",
        )
        .bind(LONG_TERM_PROJECTION_CONSUMER)
        .execute(&pool)
        .await
        .expect("seed projection state");
        let today = Utc::now().with_timezone(&Shanghai).date_naive().to_string();
        let control = LongTermProjectionWriteControl::unrestricted();
        queue_long_term_projection_daily_verify_with_control(&pool, &today, &control)
            .await
            .expect("persist daily verification pending marker and repair");
        queue_long_term_projection_daily_verify_with_control(&pool, &today, &control)
            .await
            .expect("pending daily verification is not queued twice");
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT generation FROM long_term_projection_dirty_buckets WHERE bucket_date = ?1",
            )
            .bind(&today)
            .fetch_one(&pool)
            .await
            .expect("single queued daily verification generation"),
            1
        );

        complete_long_term_projection_daily_verify_with_control(&pool, &today, true, &control)
            .await
            .expect("retention backlog keeps daily verification pending");
        assert!(
            long_term_projection_daily_verify_due(&pool)
                .await
                .expect("maintenance-pending verification remains due")
        );

        complete_long_term_projection_daily_verify_with_control(&pool, &today, false, &control)
            .await
            .expect("dirty daily verification remains pending after retention drains");
        assert!(
            long_term_projection_daily_verify_due(&pool)
                .await
                .expect("unrepaired verification remains due")
        );

        queue_long_term_projection_repairs(&pool, std::slice::from_ref(&today), "newer_generation")
            .await
            .expect("queue a newer daily repair");
        complete_long_term_projection_daily_verify_with_control(&pool, &today, false, &control)
            .await
            .expect("verified today bucket");
        assert!(
            long_term_projection_daily_verify_due(&pool)
                .await
                .expect("new dirty generation keeps verification due")
        );

        sqlx::query("DELETE FROM long_term_projection_dirty_buckets WHERE bucket_date = ?1")
            .bind(&today)
            .execute(&pool)
            .await
            .expect("clear completed dirty marker");
        complete_long_term_projection_daily_verify_with_control(&pool, &today, false, &control)
            .await
            .expect("verified clean today bucket");
        assert!(
            !long_term_projection_daily_verify_due(&pool)
                .await
                .expect("completed verification is fresh")
        );
    }

    #[tokio::test]
    async fn daily_verify_pending_bucket_survives_calendar_rollover() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        sqlx::query(
            "INSERT INTO long_term_projection_state (consumer, cursor_row_id) VALUES (?1, 0)",
        )
        .bind(LONG_TERM_PROJECTION_CONSUMER)
        .execute(&pool)
        .await
        .expect("seed projection state");
        let today = Utc::now().with_timezone(&Shanghai).date_naive();
        let yesterday = today.pred_opt().expect("previous calendar date");
        let today_text = today.to_string();
        let yesterday_text = yesterday.to_string();
        sqlx::query(
            "UPDATE long_term_projection_state SET last_daily_verify_at = ?2 WHERE consumer = ?1",
        )
        .bind(LONG_TERM_PROJECTION_CONSUMER)
        .bind(format!("{yesterday_text} 15:59:59"))
        .execute(&pool)
        .await
        .expect("seed a prior Shanghai calendar verification");
        let control = LongTermProjectionWriteControl::unrestricted();
        assert_eq!(
            queue_long_term_projection_daily_verify_with_control(&pool, &yesterday_text, &control,)
                .await
                .expect("queue the original verification bucket"),
            yesterday_text
        );
        assert_eq!(
            queue_long_term_projection_daily_verify_with_control(&pool, &today_text, &control)
                .await
                .expect("recover the original pending verification bucket after midnight"),
            yesterday_text
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT daily_verify_bucket_date FROM long_term_projection_state WHERE consumer = ?1",
            )
            .bind(LONG_TERM_PROJECTION_CONSUMER)
            .fetch_one(&pool)
            .await
            .expect("persisted pending bucket"),
            yesterday_text
        );

        sqlx::query("DELETE FROM long_term_projection_dirty_buckets WHERE bucket_date = ?1")
            .bind(&yesterday_text)
            .execute(&pool)
            .await
            .expect("simulate the original date rebuild finishing after midnight");
        complete_long_term_projection_daily_verify_with_control(
            &pool,
            &yesterday_text,
            false,
            &control,
        )
        .await
        .expect("complete only the original pending bucket");
        assert!(
            long_term_projection_daily_verify_due(&pool)
                .await
                .expect("the current Shanghai calendar day remains due")
        );
        assert_eq!(
            queue_long_term_projection_daily_verify_with_control(&pool, &today_text, &control)
                .await
                .expect("queue the unverified current calendar day"),
            today_text
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_dirty_buckets WHERE bucket_date = ?1",
            )
            .bind(&today_text)
            .fetch_one(&pool)
            .await
            .expect("current day repair marker"),
            1
        );
    }

    #[tokio::test]
    async fn bounded_ready_refresh_keeps_last_good_rows_after_a_cancelled_write() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
        let date_text = date.to_string();
        let (start_epoch, _) = long_term_day_epoch_bounds(date).expect("projection bounds");
        let mut seeded = pool.begin().await.expect("seed transaction");
        let mut daily = HashMap::new();
        for index in 0..=LONG_TERM_PROJECTION_WRITE_BATCH_ROWS {
            let series_key = format!("model:rebuilt-{index}");
            let bucket = LongTermBucket {
                bucket_start_epoch: start_epoch,
                dimension: "model".to_string(),
                series_key: series_key.clone(),
                display_name: "rebuilt".to_string(),
                reasoning_effort: String::new(),
                stats_date: Some(date_text.clone()),
                accumulator: LongTermAccumulator {
                    calls: 1,
                    ..LongTermAccumulator::default()
                },
            };
            let previous = LongTermBucket {
                series_key: format!("model:previous-{index}"),
                display_name: "previous".to_string(),
                ..bucket.clone()
            };
            insert_long_term_daily(&mut seeded, &previous)
                .await
                .expect("seed prior daily row");
            daily.insert((date_text.clone(), "model".to_string(), series_key), bucket);
        }
        seeded.commit().await.expect("commit prior daily rows");
        sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
            .bind(LONG_TERM_STATUS_READY)
            .bind(LONG_TERM_STATE_ID)
            .execute(&pool)
            .await
            .expect("seed ready long-term state");

        let shutdown = CancellationToken::new();
        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
        let committed_batches = AtomicUsize::new(0);
        let cancelling_control = LongTermProjectionWriteControl::cancelling_after(
            &shutdown,
            &gate,
            &committed_batches,
            8,
        );
        let recomputed_dates = HashSet::from([date]);
        apply_long_term_refresh_rollups_with_control(
            &pool,
            LongTermRefreshRollupInput {
                hourly: &HashMap::new(),
                daily: &daily,
                recomputed_dates: &recomputed_dates,
                retention_start: date,
                integrity_repair_failures: &[],
                completed_integrity_repairs: &HashSet::new(),
                reconstructable_start: date,
                statistics_start_date: Some(&date_text),
                initial_materialization: true,
                processed_rows_count: daily.len() as i64,
                source_rows_empty: false,
                archive_read_failed: false,
                terminal_proof_reconciliation_incomplete: false,
                archive_markers: &[],
                failed_archive_paths: &HashSet::new(),
                clear_all_attempt_markers: false,
                failed_archive_ranges: &[],
                attempt_archive_markers: &HashSet::new(),
            },
            &cancelling_control,
        )
        .await
        .expect_err("cancellation after a 512-row replacement batch");
        assert!(shutdown.is_cancelled());
        assert_eq!(committed_batches.load(Ordering::Acquire), 8);
        let interrupted_status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("interrupted status");
        assert_eq!(interrupted_status, LONG_TERM_STATUS_READY);
        let public_rows = load_long_term_daily_rows(&pool, "model", None, &date_text, &date_text)
            .await
            .expect("last-good daily rows remain readable after cancellation");
        assert_eq!(
            public_rows.len(),
            LONG_TERM_PROJECTION_WRITE_BATCH_ROWS + 1,
            "the complete last-good snapshot remains visible after cancellation"
        );
        assert!(
            public_rows
                .iter()
                .all(|row| row.series_key.starts_with("model:previous-"))
        );

        let recovery_control = LongTermProjectionWriteControl::unrestricted();
        apply_long_term_refresh_rollups_with_control(
            &pool,
            LongTermRefreshRollupInput {
                hourly: &HashMap::new(),
                daily: &daily,
                recomputed_dates: &recomputed_dates,
                retention_start: date,
                integrity_repair_failures: &[],
                completed_integrity_repairs: &HashSet::new(),
                reconstructable_start: date,
                statistics_start_date: Some(&date_text),
                initial_materialization: true,
                processed_rows_count: daily.len() as i64,
                source_rows_empty: false,
                archive_read_failed: false,
                terminal_proof_reconciliation_incomplete: false,
                archive_markers: &[],
                failed_archive_paths: &HashSet::new(),
                clear_all_attempt_markers: false,
                failed_archive_ranges: &[],
                attempt_archive_markers: &HashSet::new(),
            },
            &recovery_control,
        )
        .await
        .expect("retry initial materialization");
        let rebuilt_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_usage_daily WHERE stats_date = ?1",
        )
        .bind(&date_text)
        .fetch_one(&pool)
        .await
        .expect("rebuilt daily row count");
        assert_eq!(
            rebuilt_rows,
            (LONG_TERM_PROJECTION_WRITE_BATCH_ROWS + 1) as i64
        );
        let recovered_status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("recovered status");
        assert_eq!(recovered_status, LONG_TERM_STATUS_READY);
    }

    #[tokio::test]
    async fn cancelled_completed_repair_keeps_the_retry_queue_until_publication() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
        let date_text = date.to_string();
        let (start_epoch, _) = long_term_day_epoch_bounds(date).expect("projection bounds");
        let replacement = LongTermBucket {
            bucket_start_epoch: start_epoch,
            dimension: "model".to_string(),
            series_key: "model:replacement".to_string(),
            display_name: "replacement".to_string(),
            reasoning_effort: String::new(),
            stats_date: Some(date_text.clone()),
            accumulator: LongTermAccumulator {
                calls: 1,
                ..LongTermAccumulator::default()
            },
        };
        let daily = HashMap::from([(
            (
                date_text.clone(),
                "model".to_string(),
                replacement.series_key.clone(),
            ),
            replacement,
        )]);
        let recomputed_dates = HashSet::from([date]);
        let completed_integrity_repairs = HashSet::from([date]);
        sqlx::query(
            "INSERT INTO long_term_stats_repair_queue (stats_date, expected_calls, expected_token_total, expected_cost_total, observed_calls, observed_token_total, observed_cost_total, last_error) VALUES (?1, 1, 0, 0, 0, 0, 0, 'repair pending')",
        )
        .bind(&date_text)
        .execute(&pool)
        .await
        .expect("seed completed repair queue entry");

        let shutdown = CancellationToken::new();
        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
        let interrupted_control =
            LongTermProjectionWriteControl::stopping_after_completed_integrity_repairs(
                &shutdown, &gate,
            );
        let error = apply_long_term_refresh_rollups_with_control(
            &pool,
            LongTermRefreshRollupInput {
                hourly: &HashMap::new(),
                daily: &daily,
                recomputed_dates: &recomputed_dates,
                retention_start: date,
                integrity_repair_failures: &[],
                completed_integrity_repairs: &completed_integrity_repairs,
                reconstructable_start: date,
                statistics_start_date: Some(&date_text),
                initial_materialization: true,
                processed_rows_count: 1,
                source_rows_empty: false,
                archive_read_failed: false,
                terminal_proof_reconciliation_incomplete: false,
                archive_markers: &[],
                failed_archive_paths: &HashSet::new(),
                clear_all_attempt_markers: false,
                failed_archive_ranges: &[],
                attempt_archive_markers: &HashSet::new(),
            },
            &interrupted_control,
        )
        .await
        .expect_err("cancellation before publication");
        assert!(long_term_projection_write_is_deferred(&error));
        assert!(shutdown.is_cancelled());
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_stats_repair_queue WHERE stats_date = ?1",
            )
            .bind(&date_text)
            .fetch_one(&pool)
            .await
            .expect("queued repair survives cancellation"),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_bucket_state WHERE bucket_date = ?1 AND active_daily_backup_token IS NOT NULL AND publication_token IS NULL",
            )
            .bind(&date_text)
            .fetch_one(&pool)
            .await
            .expect("unpublished backup retains its recovery owner"),
            1
        );

        let recovery_control = LongTermProjectionWriteControl::unrestricted();
        apply_long_term_refresh_rollups_with_control(
            &pool,
            LongTermRefreshRollupInput {
                hourly: &HashMap::new(),
                daily: &daily,
                recomputed_dates: &recomputed_dates,
                retention_start: date,
                integrity_repair_failures: &[],
                completed_integrity_repairs: &completed_integrity_repairs,
                reconstructable_start: date,
                statistics_start_date: Some(&date_text),
                initial_materialization: true,
                processed_rows_count: 1,
                source_rows_empty: false,
                archive_read_failed: false,
                terminal_proof_reconciliation_incomplete: false,
                archive_markers: &[],
                failed_archive_paths: &HashSet::new(),
                clear_all_attempt_markers: false,
                failed_archive_ranges: &[],
                attempt_archive_markers: &HashSet::new(),
            },
            &recovery_control,
        )
        .await
        .expect("retry completes the queued repair");
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_stats_repair_queue WHERE stats_date = ?1",
            )
            .bind(&date_text)
            .fetch_one(&pool)
            .await
            .expect("completed repair queue entry removed with publication"),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT status FROM long_term_stats_state WHERE id = ?1",
            )
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("published state"),
            LONG_TERM_STATUS_READY
        );
    }

    #[tokio::test]
    async fn refresh_publication_commits_state_before_cleanup_can_cancel() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
        let date_text = date.to_string();
        let (start_epoch, _) = long_term_day_epoch_bounds(date).expect("projection bounds");
        let previous = LongTermBucket {
            bucket_start_epoch: start_epoch,
            dimension: "model".to_string(),
            series_key: "model:previous".to_string(),
            display_name: "previous".to_string(),
            reasoning_effort: String::new(),
            stats_date: Some(date_text.clone()),
            accumulator: LongTermAccumulator {
                calls: 1,
                ..LongTermAccumulator::default()
            },
        };
        let replacement = LongTermBucket {
            series_key: "model:replacement".to_string(),
            display_name: "replacement".to_string(),
            ..previous.clone()
        };
        let mut seeded = pool.begin().await.expect("seed transaction");
        insert_long_term_daily(&mut seeded, &previous)
            .await
            .expect("seed prior daily row");
        seeded.commit().await.expect("commit prior daily row");
        sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
            .bind(LONG_TERM_STATUS_READY)
            .bind(LONG_TERM_STATE_ID)
            .execute(&pool)
            .await
            .expect("seed ready long-term state");

        let daily = HashMap::from([(
            (
                date_text.clone(),
                "model".to_string(),
                replacement.series_key.clone(),
            ),
            replacement,
        )]);
        let recomputed_dates = HashSet::from([date]);
        let shutdown = CancellationToken::new();
        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
        let control =
            LongTermProjectionWriteControl::stopping_after_refresh_publication(&shutdown, &gate);
        let error = apply_long_term_refresh_rollups_with_control(
            &pool,
            LongTermRefreshRollupInput {
                hourly: &HashMap::new(),
                daily: &daily,
                recomputed_dates: &recomputed_dates,
                retention_start: date,
                integrity_repair_failures: &[],
                completed_integrity_repairs: &HashSet::new(),
                reconstructable_start: date,
                statistics_start_date: Some(&date_text),
                initial_materialization: true,
                processed_rows_count: 1,
                source_rows_empty: false,
                archive_read_failed: false,
                terminal_proof_reconciliation_incomplete: false,
                archive_markers: &[],
                failed_archive_paths: &HashSet::new(),
                clear_all_attempt_markers: false,
                failed_archive_ranges: &[],
                attempt_archive_markers: &HashSet::new(),
            },
            &control,
        )
        .await
        .expect_err("test cancellation stops only publication cleanup");
        assert!(long_term_projection_write_is_deferred(&error));
        assert!(shutdown.is_cancelled());
        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT status FROM long_term_stats_state WHERE id = ?1",
            )
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("published state"),
            LONG_TERM_STATUS_READY
        );
        let published_rows =
            load_long_term_daily_rows(&pool, "model", None, &date_text, &date_text)
                .await
                .expect("published replacement rows");
        assert_eq!(published_rows.len(), 1);
        assert_eq!(published_rows[0].series_key, "model:replacement");
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_bucket_state WHERE bucket_date = ?1 AND active_daily_backup_token IS NOT NULL AND publication_token IS NOT NULL",
            )
            .bind(&date_text)
            .fetch_one(&pool)
            .await
            .expect("deferred cleanup pointer"),
            1
        );

        let recovery_control = LongTermProjectionWriteControl::unrestricted();
        finish_long_term_projection_publication_cleanup(&pool, &recovery_control)
            .await
            .expect("recovery releases an already-published backup");
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_bucket_state WHERE bucket_date = ?1 AND active_daily_backup_token IS NOT NULL",
            )
            .bind(&date_text)
            .fetch_one(&pool)
            .await
            .expect("released cleanup pointer"),
            0
        );
    }

    #[tokio::test]
    async fn refresh_archive_marker_requires_the_scanned_manifest_identity() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("full schema");
        let archive_path = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-refresh-marker-{}-{}.sqlite.gz",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::write(&archive_path, b"scanned archive bytes").expect("write scanned archive");
        let archive_path = archive_path.to_string_lossy().to_string();
        let archive_sha256 =
            crate::maintenance::sha256_hex_file(std::path::Path::new(&archive_path))
                .expect("hash scanned archive");
        sqlx::query(
            "INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, created_at) VALUES ('codex_invocations', '2026-08', ?1, ?2, 1, 'completed', datetime('now'))",
        )
        .bind(&archive_path)
        .bind(&archive_sha256)
        .execute(&pool)
        .await
        .expect("record rewritten archive manifest");

        let control = LongTermProjectionWriteControl::unrestricted();
        let stale_marker = vec![(archive_path.clone(), "scanned-sha".to_string())];
        let no_failed_paths = HashSet::new();
        let no_failed_ranges = Vec::new();
        let no_attempt_markers = HashSet::new();
        persist_long_term_refresh_archive_markers_with_control(
            &pool,
            LongTermRefreshArchiveMarkers {
                archive_markers: &stale_marker,
                archive_read_failed: false,
                failed_archive_paths: &no_failed_paths,
                clear_all_attempt_markers: false,
                failed_archive_ranges: &no_failed_ranges,
                attempt_archive_markers: &no_attempt_markers,
            },
            &control,
        )
        .await
        .expect_err("stale manifest identity rejects a replay marker");
        let stale_marker_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = 'codex_invocations' AND file_path = ?2",
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .bind(&archive_path)
        .fetch_one(&pool)
        .await
        .expect("stale marker count");
        assert_eq!(stale_marker_count, 0);

        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'codex_invocations', ?2, 'prior-sha')",
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .bind(&archive_path)
        .execute(&pool)
        .await
        .expect("seed stale replay marker");
        let changed_paths = HashSet::from([archive_path.clone()]);
        persist_long_term_refresh_archive_markers_with_control(
            &pool,
            LongTermRefreshArchiveMarkers {
                archive_markers: &[],
                archive_read_failed: true,
                failed_archive_paths: &changed_paths,
                clear_all_attempt_markers: false,
                failed_archive_ranges: &no_failed_ranges,
                attempt_archive_markers: &no_attempt_markers,
            },
            &control,
        )
        .await
        .expect("clear replay marker for a changed source");
        let changed_marker_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = 'codex_invocations' AND file_path = ?2",
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .bind(&archive_path)
        .fetch_one(&pool)
        .await
        .expect("changed marker count");
        assert_eq!(changed_marker_count, 0);

        fs::write(&archive_path, b"replacement archive bytes")
            .expect("replace archive before marker persistence");
        let replacement_sha256 =
            crate::maintenance::sha256_hex_file(std::path::Path::new(&archive_path))
                .expect("hash replacement archive");
        let scanned_marker = vec![(archive_path.clone(), archive_sha256.clone())];
        persist_long_term_refresh_archive_markers_with_control(
            &pool,
            LongTermRefreshArchiveMarkers {
                archive_markers: &scanned_marker,
                archive_read_failed: false,
                failed_archive_paths: &no_failed_paths,
                clear_all_attempt_markers: false,
                failed_archive_ranges: &no_failed_ranges,
                attempt_archive_markers: &no_attempt_markers,
            },
            &control,
        )
        .await
        .expect_err("replacement before marker persistence must clear the marker");
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = 'codex_invocations' AND file_path = ?2",
            )
            .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
            .bind(&archive_path)
            .fetch_one(&pool)
            .await
            .expect("replacement marker count"),
            0
        );
        sqlx::query("UPDATE archive_batches SET sha256 = ?1 WHERE file_path = ?2")
            .bind(&replacement_sha256)
            .bind(&archive_path)
            .execute(&pool)
            .await
            .expect("update replacement manifest");
        let current_marker = vec![(archive_path.clone(), replacement_sha256.clone())];
        persist_long_term_refresh_archive_markers_with_control(
            &pool,
            LongTermRefreshArchiveMarkers {
                archive_markers: &current_marker,
                archive_read_failed: false,
                failed_archive_paths: &no_failed_paths,
                clear_all_attempt_markers: false,
                failed_archive_ranges: &no_failed_ranges,
                attempt_archive_markers: &no_attempt_markers,
            },
            &control,
        )
        .await
        .expect("persist matching archive marker");
        let persisted_sha256 = sqlx::query_scalar::<_, String>(
            "SELECT archive_sha256 FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = 'codex_invocations' AND file_path = ?2",
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .bind(&archive_path)
        .fetch_one(&pool)
        .await
        .expect("matching marker sha");
        assert_eq!(persisted_sha256, replacement_sha256);
        let _ = fs::remove_file(&archive_path);
    }

    #[tokio::test]
    async fn published_backup_cleanup_recovers_after_pointer_release_cancellation() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
        let date_text = date.to_string();
        let (start_epoch, _) = long_term_day_epoch_bounds(date).expect("projection bounds");
        let bucket = LongTermBucket {
            bucket_start_epoch: start_epoch,
            dimension: "model".to_string(),
            series_key: "model:previous".to_string(),
            display_name: "previous".to_string(),
            reasoning_effort: String::new(),
            stats_date: Some(date_text.clone()),
            accumulator: LongTermAccumulator {
                calls: 1,
                ..LongTermAccumulator::default()
            },
        };
        let mut seeded = pool.begin().await.expect("seed transaction");
        insert_long_term_daily(&mut seeded, &bucket)
            .await
            .expect("seed prior daily row");
        seeded.commit().await.expect("commit prior daily row");

        let rebuild_token = format!("long-term-date:{date_text}");
        let publication_token = "test-post-release-cleanup";
        let unrestricted = LongTermProjectionWriteControl::unrestricted();
        ensure_long_term_projection_daily_backup_for_date(
            &pool,
            &date_text,
            &rebuild_token,
            false,
            &unrestricted,
        )
        .await
        .expect("create daily backup");
        stage_long_term_projection_date_publication(
            &pool,
            &date_text,
            &rebuild_token,
            publication_token,
            None,
            &unrestricted,
        )
        .await
        .expect("stage publication");
        sqlx::query(
            "INSERT INTO long_term_projection_date_publications (publication_token, published) VALUES (?1, 1)",
        )
        .bind(publication_token)
        .execute(&pool)
        .await
        .expect("publish replacement");

        let shutdown = CancellationToken::new();
        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
        let cancelling =
            LongTermProjectionWriteControl::stopping_after_backup_cleanup_marker(&shutdown, &gate);
        let error = release_long_term_projection_publication_member(
            &pool,
            &date_text,
            &rebuild_token,
            None,
            publication_token,
            &cancelling,
        )
        .await
        .expect_err("cancellation follows the durable cleanup marker");
        assert!(long_term_projection_write_is_deferred(&error));
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_bucket_state WHERE bucket_date = ?1 AND active_daily_backup_token IS NULL",
            )
            .bind(&date_text)
            .fetch_one(&pool)
            .await
            .expect("released backup pointer"),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT publication_token FROM long_term_projection_bucket_state WHERE bucket_date = ?1",
            )
            .bind(&date_text)
            .fetch_one(&pool)
            .await
            .expect("durable cleanup marker"),
            Some(format!("cleanup:{rebuild_token}"))
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_daily_backups WHERE rebuild_token = ?1",
            )
            .bind(&rebuild_token)
            .fetch_one(&pool)
            .await
            .expect("orphaned backup before recovery"),
            1
        );

        assert!(
            finish_long_term_projection_publication_cleanup(&pool, &unrestricted)
                .await
                .expect("remove one bounded backup cleanup batch")
        );
        assert!(
            finish_long_term_projection_publication_cleanup(&pool, &unrestricted)
                .await
                .expect("clear one bounded cleanup marker batch")
        );
        assert_eq!(
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT publication_token FROM long_term_projection_bucket_state WHERE bucket_date = ?1",
            )
            .bind(&date_text)
            .fetch_one(&pool)
            .await
            .expect("cleared cleanup marker"),
            None
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_daily_backups WHERE rebuild_token = ?1",
            )
            .bind(&rebuild_token)
            .fetch_one(&pool)
            .await
            .expect("cleared orphaned backup"),
            0
        );
    }

    #[tokio::test]
    async fn publication_cleanup_advances_one_cancelable_512_row_batch_per_maintenance_pass() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        let rebuild_token = "bounded-publication-cleanup";
        for index in 0..1_025 {
            let bucket_date = format!("cleanup-date-{index:04}");
            sqlx::query(
                "INSERT INTO long_term_projection_bucket_state (bucket_date, publication_token) VALUES (?1, ?2)",
            )
            .bind(&bucket_date)
            .bind(format!("cleanup:{rebuild_token}"))
            .execute(&pool)
            .await
            .expect("seed cleanup marker");
            sqlx::query(
                "INSERT INTO long_term_projection_daily_backups (rebuild_token, stats_date, dimension, series_key, display_name) VALUES (?1, ?2, 'overall', 'overall', 'All')",
            )
            .bind(rebuild_token)
            .bind(bucket_date)
            .execute(&pool)
            .await
            .expect("seed backup cleanup row");
        }
        let control = LongTermProjectionWriteControl::unrestricted();
        assert!(
            finish_long_term_projection_publication_cleanup(&pool, &control)
                .await
                .expect("first bounded maintenance cleanup batch")
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_daily_backups WHERE rebuild_token = ?1",
            )
            .bind(rebuild_token)
            .fetch_one(&pool)
            .await
            .expect("one bounded backup batch remaining"),
            513
        );
        assert!(
            long_term_projection_maintenance_needed(&pool, 366)
                .await
                .expect("durable cleanup backlog schedules the next maintenance pass")
        );

        let shutdown = CancellationToken::new();
        shutdown.cancel();
        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
        let cancelled = LongTermProjectionWriteControl::background(&shutdown, &gate);
        let error = finish_long_term_projection_publication_cleanup(&pool, &cancelled)
            .await
            .expect_err("cancellation stops before the next cleanup write");
        assert!(error.to_string().contains("cancelled"));
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_daily_backups WHERE rebuild_token = ?1",
            )
            .bind(rebuild_token)
            .fetch_one(&pool)
            .await
            .expect("cancelled cleanup preserved the durable backlog"),
            513
        );

        while finish_long_term_projection_publication_cleanup(&pool, &control)
            .await
            .expect("resume one bounded cleanup batch at a time")
        {}
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_daily_backups WHERE rebuild_token = ?1",
            )
            .bind(rebuild_token)
            .fetch_one(&pool)
            .await
            .expect("drained cleanup backups"),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_bucket_state WHERE publication_token IS NOT NULL",
            )
            .fetch_one(&pool)
            .await
            .expect("drained cleanup markers"),
            0
        );
        assert!(
            !long_term_projection_maintenance_needed(&pool, 366)
                .await
                .expect("drained cleanup no longer schedules maintenance")
        );
    }

    #[tokio::test]
    async fn interrupted_initial_refresh_retries_empty_source_without_publishing_partial_rollups() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("full schema");
        let date = Utc::now().with_timezone(&Shanghai).date_naive().to_string();
        sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name) VALUES (?1, 'overall', 'overall', 'last good')",
        )
        .bind(&date)
        .execute(&pool)
        .await
        .expect("seed partial durable row");

        let shutdown = CancellationToken::new();
        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
        let committed_batches = AtomicUsize::new(0);
        let control = LongTermProjectionWriteControl::cancelling_after(
            &shutdown,
            &gate,
            &committed_batches,
            1,
        );
        refresh_long_term_stats_once(&pool, 400, &control)
            .await
            .expect_err("cancelled initial refresh");
        assert!(shutdown.is_cancelled());

        let state: (String, Option<String>) =
            sqlx::query_as("SELECT status, last_error FROM long_term_stats_state WHERE id = ?1")
                .bind(LONG_TERM_STATE_ID)
                .fetch_one(&pool)
                .await
                .expect("interrupted initial state");
        assert_eq!(state.0, LONG_TERM_STATUS_RUNNING);
        assert_eq!(
            state.1.as_deref(),
            Some(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR)
        );
        assert!(long_term_initial_materialization_needed(
            &state.0,
            state.1.as_deref(),
            true,
        ));

        let recovery_control = LongTermProjectionWriteControl::unrestricted();
        refresh_long_term_stats_once(&pool, 400, &recovery_control)
            .await
            .expect("empty-source retry clears its interrupted initial prefix");
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_usage_daily")
                .fetch_one(&pool)
                .await
                .expect("empty-source retry daily rows"),
            0
        );
        let recovered_state: (String, Option<String>) =
            sqlx::query_as("SELECT status, last_error FROM long_term_stats_state WHERE id = ?1")
                .bind(LONG_TERM_STATE_ID)
                .fetch_one(&pool)
                .await
                .expect("empty-source recovery state");
        assert_eq!(recovered_state.0, LONG_TERM_STATUS_EMPTY);
        assert_eq!(recovered_state.1, None);
    }

    #[tokio::test]
    async fn initial_refresh_retries_after_missing_attempt_archive_recovers() {
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
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, model, payload, raw_response, total_tokens, output_tokens, cost, created_at) VALUES (1, 'initial-missing-attempt-source', ?1, 'success', 'gpt-5', '{}', '{}', 100, 40, 0.1, datetime('now'))",
        )
        .bind(&occurred_at)
        .execute(&pool)
        .await
        .expect("insert live invocation");
        let missing_path = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-initial-missing-attempt-source-{}-{}.sqlite.gz",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        let missing_path = missing_path.to_string_lossy().to_string();
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                dataset, month_key, file_path, sha256, row_count, status,
                coverage_start_at, coverage_end_at, created_at
            )
            VALUES ('pool_upstream_request_attempts', ?1, ?2, 'initial-missing-attempt-sha',
                1, 'completed', ?3, ?3, datetime('now'))
            "#,
        )
        .bind(date.format("%Y-%m").to_string())
        .bind(&missing_path)
        .bind(&occurred_at)
        .execute(&pool)
        .await
        .expect("record missing attempt archive manifest");

        let error = refresh_long_term_stats(&pool, 400)
            .await
            .expect_err("missing initial archive keeps materialization retryable");
        assert!(
            error
                .to_string()
                .contains(LONG_TERM_ATTEMPT_ARCHIVE_UNAVAILABLE_ERROR)
        );
        let initial_state = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT status, last_error FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("initial archive failure state");
        assert_eq!(initial_state.0, LONG_TERM_STATUS_ERROR);
        assert!(
            initial_state.1.as_deref().is_some_and(
                |message| message.contains(LONG_TERM_ATTEMPT_ARCHIVE_UNAVAILABLE_ERROR)
            )
        );

        sqlx::query("DELETE FROM archive_batches WHERE file_path = ?1")
            .bind(&missing_path)
            .execute(&pool)
            .await
            .expect("remove recovered archive manifest");
        refresh_long_term_stats(&pool, 400)
            .await
            .expect("initial materialization retries after archive recovery");
        let recovered_state = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT status, last_error FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("recovered initial state");
        assert_eq!(recovered_state.0, LONG_TERM_STATUS_READY);
        assert_eq!(recovered_state.1, None);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
            )
            .bind(date.to_string())
            .fetch_one(&pool)
            .await
            .expect("recovered daily rollup"),
            1
        );
    }

    #[tokio::test]
    async fn initial_refresh_replays_later_archives_after_an_earlier_source_recovers() {
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
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let missing_archive_path = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-initial-missing-invocation-{suffix}.sqlite.gz"
        ));
        let readable_db_path = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-initial-readable-invocation-{suffix}.sqlite"
        ));
        let readable_archive_path = readable_db_path.with_extension("sqlite.gz");
        fs::File::create(&readable_db_path).expect("create readable invocation archive database");
        let archive_options = format!("sqlite://{}", readable_db_path.to_string_lossy())
            .parse::<SqliteConnectOptions>()
            .expect("parse readable invocation archive URL")
            .create_if_missing(true);
        let archive_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(archive_options)
            .await
            .expect("open readable invocation archive database");
        sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, invoke_id TEXT, occurred_at TEXT NOT NULL, status TEXT, model TEXT, total_tokens INTEGER, output_tokens INTEGER, cost REAL)",
        )
        .execute(&archive_pool)
        .await
        .expect("create readable invocation archive schema");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, model, total_tokens, output_tokens, cost) VALUES (2, 'later-readable-archive', ?1, 'success', 'gpt-5', 100, 40, 0.1)",
        )
        .bind(&occurred_at)
        .execute(&archive_pool)
        .await
        .expect("insert later readable invocation archive row");
        archive_pool.close().await;
        crate::maintenance::deflate_sqlite_file_to_gzip(&readable_db_path, &readable_archive_path)
            .expect("compress readable invocation archive");
        let readable_archive_sha256 = crate::maintenance::sha256_hex_file(&readable_archive_path)
            .expect("hash readable invocation archive");

        for (id, file_path, sha256) in [
            (
                1_i64,
                missing_archive_path.to_string_lossy().to_string(),
                "missing-invocation-sha".to_string(),
            ),
            (
                2_i64,
                readable_archive_path.to_string_lossy().to_string(),
                readable_archive_sha256,
            ),
        ] {
            sqlx::query(
                r#"
                INSERT INTO archive_batches (
                    id, dataset, month_key, file_path, sha256, row_count, status,
                    coverage_start_at, coverage_end_at, created_at
                )
                VALUES (?1, 'codex_invocations', ?2, ?3, ?4, 1, 'completed', ?5, ?5, datetime('now'))
                "#,
            )
            .bind(id)
            .bind(date.format("%Y-%m").to_string())
            .bind(file_path)
            .bind(sha256)
            .bind(&occurred_at)
            .execute(&pool)
            .await
            .expect("record invocation archive manifest");
        }

        refresh_long_term_stats(&pool, 400)
            .await
            .expect("unreadable initial archive leaves a retryable state");
        let initial_state = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
            "SELECT status, last_error, statistics_start_date FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("initial archive failure state");
        assert_eq!(initial_state.0, LONG_TERM_STATUS_ERROR);
        assert_eq!(
            initial_state.1.as_deref(),
            Some(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR)
        );
        assert!(
            initial_state.2.is_some(),
            "later readable archive reached a provisional start date"
        );

        let restored_db_path = missing_archive_path.with_extension("sqlite");
        fs::File::create(&restored_db_path).expect("create restored invocation archive database");
        let restored_options = format!("sqlite://{}", restored_db_path.to_string_lossy())
            .parse::<SqliteConnectOptions>()
            .expect("parse restored invocation archive URL")
            .create_if_missing(true);
        let restored_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(restored_options)
            .await
            .expect("open restored invocation archive database");
        sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, invoke_id TEXT, occurred_at TEXT NOT NULL, status TEXT, model TEXT, total_tokens INTEGER, output_tokens INTEGER, cost REAL)",
        )
        .execute(&restored_pool)
        .await
        .expect("create restored invocation archive schema");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, model, total_tokens, output_tokens, cost) VALUES (1, 'restored-earlier-archive', ?1, 'success', 'gpt-5', 100, 40, 0.1)",
        )
        .bind(&occurred_at)
        .execute(&restored_pool)
        .await
        .expect("insert restored invocation archive row");
        restored_pool.close().await;
        crate::maintenance::deflate_sqlite_file_to_gzip(&restored_db_path, &missing_archive_path)
            .expect("compress restored invocation archive");
        let restored_archive_sha256 = crate::maintenance::sha256_hex_file(&missing_archive_path)
            .expect("hash restored invocation archive");
        sqlx::query("UPDATE archive_batches SET sha256 = ?1 WHERE file_path = ?2")
            .bind(restored_archive_sha256)
            .bind(missing_archive_path.to_string_lossy().to_string())
            .execute(&pool)
            .await
            .expect("restore invocation archive manifest identity");

        refresh_long_term_stats(&pool, 400)
            .await
            .expect("initial recovery must replay every invocation archive");
        let recovered_state = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT status, last_error FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("recovered initial state");
        assert_eq!(recovered_state.0, LONG_TERM_STATUS_READY);
        assert_eq!(recovered_state.1, None);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
            )
            .bind(date.to_string())
            .fetch_one(&pool)
            .await
            .expect("recovered daily rollup"),
            2,
            "the later archive must not be skipped because its first replay marker survived the failed pass"
        );

        let _ = fs::remove_file(&readable_db_path);
        let _ = fs::remove_file(&readable_archive_path);
        let _ = fs::remove_file(&restored_db_path);
        let _ = fs::remove_file(&missing_archive_path);
    }

    #[test]
    fn repeated_terminal_defers_preserve_the_original_repair_deadline() {
        let now = Instant::now();
        let original = now + Duration::from_secs(30);
        assert_eq!(
            long_term_projection_repair_deadline(Some(original), now),
            original
        );
        let overdue = now - Duration::from_secs(1);
        assert_eq!(
            long_term_projection_repair_deadline(Some(overdue), now),
            overdue
        );
    }

    #[test]
    fn terminal_projection_flush_does_not_share_the_repair_backoff() {
        assert!(long_term_projection_terminal_flush_due(true, false));
        assert!(long_term_projection_terminal_flush_due(false, true));
        assert!(!long_term_projection_terminal_flush_due(false, false));
    }

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
                reasoning_tokens INTEGER,
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

    #[tokio::test]
    async fn terminal_projection_seek_uses_terminal_id_index_after_pending_prefix() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        create_long_term_test_invocations(&pool).await;
        sqlx::query(
            r#"
            WITH RECURSIVE source(id) AS (
                VALUES(1)
                UNION ALL
                SELECT id + 1 FROM source WHERE id < 1025
            )
            INSERT INTO codex_invocations (id, occurred_at, status)
            SELECT
                id,
                printf('2026-07-%02d 00:00:00', (id % 28) + 1),
                CASE WHEN id = 1025 THEN 'success' ELSE 'running' END
            FROM source
            "#,
        )
        .execute(&pool)
        .await
        .expect("sparse terminal source rows");
        ensure_long_term_projection_source_indexes(&pool)
            .await
            .expect("projection source indexes");
        let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(
            "EXPLAIN QUERY PLAN SELECT id FROM codex_invocations WHERE id > ?1 AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending') ORDER BY id ASC LIMIT ?2",
        )
        .bind(0_i64)
        .bind(LONG_TERM_PROJECTION_MAX_EVENTS_PER_FLUSH)
        .fetch_all(&pool)
        .await
        .expect("terminal seek query plan");
        assert!(plan.iter().any(|(_, _, _, detail)| {
            detail.contains("idx_codex_invocations_long_term_projection_terminal_id")
                && detail.contains("id>?")
        }));
        let rows = load_long_term_projection_terminal_rows(&pool, 0)
            .await
            .expect("terminal projection seek");
        assert_eq!(
            rows.into_iter().map(|row| row.id).collect::<Vec<_>>(),
            vec![1025]
        );
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
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_input_tokens INTEGER NOT NULL DEFAULT 0,
                reasoning_tokens INTEGER NOT NULL DEFAULT 0,
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

    async fn long_term_file_backed_pool_with_busy_timeout(
        prefix: &str,
        busy_timeout: Duration,
    ) -> (Pool<Sqlite>, String, PathBuf) {
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
            .busy_timeout(busy_timeout);
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

    async fn long_term_file_backed_pool(prefix: &str) -> (Pool<Sqlite>, String, PathBuf) {
        long_term_file_backed_pool_with_busy_timeout(prefix, Duration::from_millis(50)).await
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
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, model, payload, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens, cost) VALUES (?1, ?2, ?3, 'success', 'gpt-5', '{}', 60, 40, 0, 0, 100, 0.1)",
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
                "INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, status, model, payload, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens, cost) VALUES (?1, ?2, ?3, ?4, 'success', 'gpt-5', '{\"reasoningEffort\":\"high\"}', 60, 40, 0, 0, 100, 0.1)",
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
        let archive_sha256 =
            crate::maintenance::sha256_hex_file(&archive_path).expect("hash attempt archive");
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
            VALUES ('pool_upstream_request_attempts', ?1, ?2, ?3, 1, 'completed', ?4, ?4, datetime('now'))
            "#,
        )
        .bind(month_key)
        .bind(archive_path.to_string_lossy().to_string())
        .bind(archive_sha256)
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
        assert_eq!(
            long_term_refresh_pending_marker(true, LONG_TERM_STATUS_READY),
            None
        );
        assert_eq!(
            long_term_refresh_pending_marker(false, LONG_TERM_STATUS_RUNNING),
            Some(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR)
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
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, model, payload, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens, cost) VALUES (2, 'active', ?1, 'running', 'gpt-5', '{}', 60, 40, 0, 0, 100, 0.1)",
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
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, detail_level, model, payload, raw_response, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens, cost) VALUES ('reconciliation-source', ?1, ?2, 'success', 'full', 'gpt-5', '{}', '{}', 60, 40, 0, 0, 100, 0.1)",
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
                .expect("prune one bounded hourly retention batch");

        assert_eq!(pruned_hourly_rows, 1);
        assert_eq!(pruned_interval_rows, 0);
        let (pruned_hourly_rows, pruned_interval_rows) =
            prune_long_term_projection_hourly_retention(&pool, 366)
                .await
                .expect("prune one bounded legacy retention batch");
        assert_eq!(pruned_hourly_rows, 0);
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

    fn projection_interval_segment(
        invocation_row_id: i64,
        interval_start_ms: i64,
        interval_end_ms: i64,
    ) -> LongTermProjectionIntervalSegment {
        LongTermProjectionIntervalSegment {
            invocation_row_id,
            model_series_key: "model:gpt-5:high".to_string(),
            upstream_series_key: "account:42".to_string(),
            interval_start_ms,
            interval_end_ms,
        }
    }

    #[tokio::test]
    async fn projection_interval_state_compacts_each_invocation_and_survives_reopen() {
        let (pool, db_url, db_path) = long_term_file_backed_pool("projection-interval-state").await;
        let start_ms = Shanghai
            .with_ymd_and_hms(2026, 7, 26, 10, 0, 0)
            .single()
            .expect("Shanghai start")
            .timestamp_millis();
        let segments = (1..=256)
            .map(|id| projection_interval_segment(id, start_ms, start_ms + 1_000))
            .collect::<Vec<_>>();
        let control = LongTermProjectionWriteControl::unrestricted();
        upsert_long_term_projection_interval_segments(&pool, &segments, &control)
            .await
            .expect("canonical interval state");
        let state_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_projection_interval_state",
        )
        .fetch_one(&pool)
        .await
        .expect("canonical state row count");
        assert_eq!(state_rows, 256);
        let runtime = Arc::new(Mutex::new(LongTermProjectionRuntime::default()));
        apply_long_term_projection_incremental_with_runtime(
            &pool,
            &runtime,
            &HashMap::new(),
            &HashMap::new(),
            &[],
            321,
            0,
        )
        .await
        .expect("persist projection cursor with canonical state");
        pool.close().await;

        let reopened = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&db_url)
            .await
            .expect("reopen durable projection state");
        let dates = HashSet::from(["2026-07-26".to_string()]);
        let index = load_long_term_projection_interval_index(&reopened, &dates)
            .await
            .expect("load canonical interval index");
        let key = projection_interval_key(
            "daily",
            "2026-07-26".to_string(),
            "model".to_string(),
            "model:gpt-5:high".to_string(),
        );
        assert_eq!(
            index.get(&key).expect("model daily union").duration_ms,
            1_000
        );
        assert_eq!(
            load_long_term_projection_cursor(&reopened)
                .await
                .expect("reopened projection cursor"),
            321
        );
        cleanup_long_term_file_backed_pool(reopened, db_path).await;
    }

    #[tokio::test]
    async fn incremental_projection_reuses_interval_index_across_micro_batches() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
        let (start_epoch, _) = long_term_day_epoch_bounds(date).expect("projection bounds");
        let start_ms = start_epoch * 1_000;
        let control = LongTermProjectionWriteControl::unrestricted();
        upsert_long_term_projection_interval_segments(
            &pool,
            &[projection_interval_segment(1, start_ms, start_ms + 1_000)],
            &control,
        )
        .await
        .expect("seed existing interval state");
        let runtime = Arc::new(Mutex::new(LongTermProjectionRuntime::default()));
        let first_segment = projection_interval_segment(2, start_ms + 1_000, start_ms + 2_000);
        let first_outcome = apply_long_term_projection_incremental_with_runtime_and_control(
            &pool,
            &runtime,
            LongTermProjectionIncrementalBatch {
                hourly: &HashMap::new(),
                daily: &HashMap::new(),
                segments: std::slice::from_ref(&first_segment),
            },
            2,
            1,
            &control,
        )
        .await
        .expect("first incremental micro-batch");
        assert_eq!(
            first_outcome,
            LongTermProjectionIncrementalOutcome::Published
        );

        // A same-date batch must reuse the loaded union. Clearing the backing rows makes an
        // accidental reload observable while leaving the in-memory cache unchanged.
        sqlx::query("DELETE FROM long_term_projection_interval_state")
            .execute(&pool)
            .await
            .expect("remove persisted rows after first cache load");
        let second_segment = projection_interval_segment(3, start_ms + 2_000, start_ms + 3_000);
        let second_outcome = apply_long_term_projection_incremental_with_runtime_and_control(
            &pool,
            &runtime,
            LongTermProjectionIncrementalBatch {
                hourly: &HashMap::new(),
                daily: &HashMap::new(),
                segments: std::slice::from_ref(&second_segment),
            },
            3,
            1,
            &control,
        )
        .await
        .expect("second incremental micro-batch");
        assert_eq!(
            second_outcome,
            LongTermProjectionIncrementalOutcome::Published
        );
        let runtime = runtime.lock().await;
        let daily_key = projection_interval_key(
            "daily",
            date.to_string(),
            "overall".to_string(),
            "overall".to_string(),
        );
        assert_eq!(
            runtime
                .interval_index
                .get(&daily_key)
                .expect("same-date cache entry")
                .duration_ms,
            3_000,
            "the second micro-batch must not discard the prior same-date union"
        );
    }

    #[tokio::test]
    async fn canonical_interval_lookup_seeks_from_interval_end() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
        let (start_epoch, end_epoch) = long_term_day_epoch_bounds(date).expect("projection bounds");
        let start_ms = start_epoch * 1_000;
        let end_ms = end_epoch * 1_000;
        sqlx::query(
            "INSERT INTO long_term_projection_interval_state (invocation_row_id, model_series_key, upstream_series_key, interval_start_ms, interval_end_ms) VALUES (1, 'model:old', 'account:old', ?1, ?2), (2, 'model:current', 'account:current', ?3, ?4)",
        )
        .bind(start_ms - 86_400_000)
        .bind(start_ms - 1)
        .bind(start_ms + 1_000)
        .bind(start_ms + 2_000)
        .execute(&pool)
        .await
        .expect("seed canonical intervals");
        let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(
            "EXPLAIN QUERY PLAN SELECT invocation_row_id FROM long_term_projection_interval_state WHERE interval_start_ms < ?1 AND interval_end_ms > ?2",
        )
        .bind(end_ms)
        .bind(start_ms)
        .fetch_all(&pool)
        .await
        .expect("canonical interval query plan");
        assert!(plan.iter().any(|(_, _, _, detail)| {
            detail.contains("idx_long_term_projection_interval_state_end_start")
        }));

        let index =
            load_long_term_projection_interval_index(&pool, &HashSet::from([date.to_string()]))
                .await
                .expect("load canonical interval index");
        let key = projection_interval_key(
            "daily",
            date.to_string(),
            "model".to_string(),
            "model:current".to_string(),
        );
        assert_eq!(
            index.get(&key).expect("current interval union").duration_ms,
            1_000
        );
    }

    #[tokio::test]
    async fn legacy_interval_migration_seeks_by_invocation_id() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(
            r#"
            EXPLAIN QUERY PLAN
            SELECT legacy.invocation_row_id,
                   MAX(CASE WHEN legacy.dimension = 'model' THEN legacy.series_key END),
                   MAX(CASE WHEN legacy.dimension = 'upstream' THEN legacy.series_key END),
                   MIN(legacy.interval_start_ms),
                   MAX(legacy.interval_end_ms)
            FROM long_term_projection_intervals legacy
            WHERE NOT EXISTS (
                SELECT 1
                FROM long_term_projection_interval_state state
                WHERE state.invocation_row_id = legacy.invocation_row_id
            )
            GROUP BY legacy.invocation_row_id
            ORDER BY legacy.invocation_row_id ASC
            LIMIT 512
            "#,
        )
        .fetch_all(&pool)
        .await
        .expect("legacy migration query plan");
        assert!(plan.iter().any(|(_, _, _, detail)| {
            detail.contains("idx_long_term_projection_intervals_invocation")
        }));
    }

    #[tokio::test]
    async fn legacy_interval_fallback_honors_rebuild_suppressions() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        let date = "2026-07-26";
        sqlx::query(
            "INSERT INTO long_term_projection_intervals (bucket_kind, bucket_date, bucket_key, dimension, series_key, invocation_row_id, interval_start_ms, interval_end_ms) VALUES ('daily', ?1, ?1, 'overall', 'overall', 77, 1784992800000, 1784992801000)",
        )
        .bind(date)
        .execute(&pool)
        .await
        .expect("seed legacy-only interval");
        sqlx::query(
            "INSERT INTO long_term_projection_interval_suppressions (invocation_row_id, bucket_date) VALUES (77, ?1)",
        )
        .bind(date)
        .execute(&pool)
        .await
        .expect("suppress removed legacy interval");

        let index =
            load_long_term_projection_interval_index(&pool, &HashSet::from([date.to_string()]))
                .await
                .expect("load legacy fallback");
        assert!(
            index.is_empty(),
            "suppressed legacy state must not resurrect"
        );
    }

    #[tokio::test]
    async fn projection_interval_state_retention_prunes_canonical_state_and_metadata() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        let retention_start = long_term_projection_hourly_retention_start_date(366);
        let old_date = retention_start.pred_opt().expect("old retention date");
        let old_start_ms = Shanghai
            .from_local_datetime(&old_date.and_hms_opt(0, 0, 0).expect("old start"))
            .single()
            .expect("old Shanghai start")
            .timestamp_millis();
        let retained_start_ms = Shanghai
            .from_local_datetime(
                &retention_start
                    .and_hms_opt(0, 0, 0)
                    .expect("retention start"),
            )
            .single()
            .expect("retained Shanghai start")
            .timestamp_millis();
        let control = LongTermProjectionWriteControl::unrestricted();
        upsert_long_term_projection_interval_segments(
            &pool,
            &[
                projection_interval_segment(1, old_start_ms, old_start_ms + 1_000),
                projection_interval_segment(2, retained_start_ms, retained_start_ms + 1_000),
            ],
            &control,
        )
        .await
        .expect("seed canonical state");
        sqlx::query(
            "INSERT INTO long_term_projection_interval_suppressions (invocation_row_id, bucket_date) VALUES (1, ?1)",
        )
        .bind(old_date.to_string())
        .execute(&pool)
        .await
        .expect("old suppression");
        sqlx::query(
            "INSERT INTO long_term_projection_rebuild_members (rebuild_token, invocation_row_id) VALUES ('old-retention', 1)",
        )
        .execute(&pool)
        .await
        .expect("old rebuild membership");

        let mut pruned_intervals = 0;
        loop {
            let (_, pruned) =
                prune_long_term_projection_hourly_retention_with_control(&pool, 366, &control)
                    .await
                    .expect("bounded retention pass");
            if pruned == 0 {
                break;
            }
            assert!(pruned <= LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as u64);
            pruned_intervals += pruned;
        }
        assert_eq!(pruned_intervals, 3);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_interval_state WHERE invocation_row_id = 1",
            )
            .fetch_one(&pool)
            .await
            .expect("old state count"),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_interval_state WHERE invocation_row_id = 2",
            )
            .fetch_one(&pool)
            .await
            .expect("retained state count"),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_interval_suppressions",
            )
            .fetch_one(&pool)
            .await
            .expect("suppression cleanup"),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_rebuild_members",
            )
            .fetch_one(&pool)
            .await
            .expect("member cleanup"),
            0
        );
    }

    #[tokio::test]
    async fn projection_interval_retention_batches_defer_cancel_and_resume() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        let retention_start = long_term_projection_hourly_retention_start_date(366);
        let expired_date = retention_start.pred_opt().expect("expired retention date");
        let expired_start_ms = Shanghai
            .from_local_datetime(&expired_date.and_hms_opt(0, 0, 0).expect("expired start"))
            .single()
            .expect("expired Shanghai start")
            .timestamp_millis();
        let retained_start_ms = Shanghai
            .from_local_datetime(
                &retention_start
                    .and_hms_opt(0, 0, 0)
                    .expect("retained start"),
            )
            .single()
            .expect("retained Shanghai start")
            .timestamp_millis();
        let mut segments = (1..=1_025)
            .map(|id| projection_interval_segment(id, expired_start_ms, expired_start_ms + 1_000))
            .collect::<Vec<_>>();
        segments.push(projection_interval_segment(
            2_000,
            retained_start_ms,
            retained_start_ms + 1_000,
        ));
        let unrestricted = LongTermProjectionWriteControl::unrestricted();
        upsert_long_term_projection_interval_segments(&pool, &segments, &unrestricted)
            .await
            .expect("seed canonical interval state");
        for invocation_row_id in 1..=1_025 {
            sqlx::query(
                "INSERT INTO long_term_projection_intervals (bucket_kind, bucket_date, bucket_key, dimension, series_key, invocation_row_id, interval_start_ms, interval_end_ms) VALUES ('hourly', ?1, ?2, 'overall', 'overall', ?3, ?4, ?5)",
            )
            .bind(expired_date.to_string())
            .bind(format!("retention-{invocation_row_id}"))
            .bind(invocation_row_id)
            .bind(expired_start_ms)
            .bind(expired_start_ms + 1_000)
            .execute(&pool)
            .await
            .expect("seed expanded legacy interval");
            sqlx::query(
                "INSERT INTO long_term_projection_interval_suppressions (invocation_row_id, bucket_date) VALUES (?1, ?2)",
            )
            .bind(invocation_row_id)
            .bind(expired_date.to_string())
            .execute(&pool)
            .await
            .expect("seed expired suppression");
            sqlx::query(
                "INSERT INTO long_term_projection_rebuild_members (rebuild_token, invocation_row_id) VALUES (?1, ?2)",
            )
            .bind(format!("retention-{invocation_row_id}"))
            .bind(invocation_row_id)
            .execute(&pool)
            .await
            .expect("seed expired rebuild member");
        }

        let pressure_shutdown = CancellationToken::new();
        let pressure_gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
        let held = pressure_gate
            .try_begin_background("retention-test-holder")
            .expect("hold background admission");
        let pressure_control =
            LongTermProjectionWriteControl::background(&pressure_shutdown, &pressure_gate);
        let deferred =
            prune_long_term_projection_hourly_retention_with_control(&pool, 366, &pressure_control)
                .await
                .expect_err(
                    "retention must defer before opening a write transaction under pressure",
                );
        assert!(long_term_projection_write_is_deferred(&deferred));
        drop(held);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_interval_state WHERE invocation_row_id <= 1025",
            )
            .fetch_one(&pool)
            .await
            .expect("canonical rows remain after pressure deferral"),
            1_025
        );

        let shutdown = CancellationToken::new();
        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
        let committed_batches = AtomicUsize::new(0);
        let interrupted =
            LongTermProjectionWriteControl::stopping_after(&shutdown, &gate, &committed_batches, 1);
        let (_, pruned_intervals) =
            prune_long_term_projection_hourly_retention_with_control(&pool, 366, &interrupted)
                .await
                .expect("one bounded retention batch commits before cancellation");
        assert_eq!(
            pruned_intervals,
            LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as u64
        );
        assert_eq!(committed_batches.load(Ordering::Acquire), 1);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_interval_state WHERE invocation_row_id <= 1025",
            )
            .fetch_one(&pool)
            .await
                .expect("canonical rows remain before their retention turn"),
            1_025
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_intervals")
                .fetch_one(&pool)
                .await
                .expect("one legacy retention batch leaves durable work for the next pass"),
            513
        );
        assert!(
            long_term_projection_maintenance_needed(&pool, 366)
                .await
                .expect("durable maintenance backlog remains scheduled after one batch")
        );
        let error =
            prune_long_term_projection_hourly_retention_with_control(&pool, 366, &interrupted)
                .await
                .expect_err("cancellation stops the next maintenance pass before its write");
        assert!(error.to_string().contains("cancelled"));

        let mut resumed_pruned_intervals = 0;
        loop {
            let (_, pruned) =
                prune_long_term_projection_hourly_retention_with_control(&pool, 366, &unrestricted)
                    .await
                    .expect("retention resumes after cancellation");
            if pruned == 0 {
                break;
            }
            assert!(pruned <= LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as u64);
            resumed_pruned_intervals += pruned;
        }
        assert_eq!(resumed_pruned_intervals, 3_588);
        assert!(
            !long_term_projection_maintenance_needed(&pool, 366)
                .await
                .expect("drained maintenance no longer schedules a tick")
        );
        for table in [
            "long_term_projection_interval_state",
            "long_term_projection_interval_suppressions",
            "long_term_projection_rebuild_members",
        ] {
            assert_eq!(
                sqlx::query_scalar::<_, i64>(&format!(
                    "SELECT COUNT(*) FROM {table} WHERE invocation_row_id <= 1025"
                ))
                .fetch_one(&pool)
                .await
                .expect("expired retention rows removed after resume"),
                0,
                "{table} must resume bounded cleanup"
            );
        }
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_interval_state WHERE invocation_row_id = 2000",
            )
            .fetch_one(&pool)
            .await
            .expect("retained canonical row"),
            1
        );
    }

    #[tokio::test]
    async fn projection_interval_state_batches_stop_for_cancel_and_pressure() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_projection_schema(&pool)
            .await
            .expect("projection schema");
        let start_ms = Shanghai
            .with_ymd_and_hms(2026, 7, 26, 10, 0, 0)
            .single()
            .expect("Shanghai start")
            .timestamp_millis();
        let segments = (1..=1_025)
            .map(|id| projection_interval_segment(id, start_ms, start_ms + 1_000))
            .collect::<Vec<_>>();
        let shutdown = CancellationToken::new();
        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
        let committed_batches = AtomicUsize::new(0);
        let cancel_control = LongTermProjectionWriteControl::cancelling_after(
            &shutdown,
            &gate,
            &committed_batches,
            1,
        );
        let cancelled =
            upsert_long_term_projection_interval_segments(&pool, &segments, &cancel_control)
                .await
                .expect_err("shutdown cancels the next internal write batch");
        assert!(cancelled.to_string().contains("cancelled"));
        assert!(shutdown.is_cancelled());
        assert_eq!(committed_batches.load(Ordering::Acquire), 1);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_interval_state",
            )
            .fetch_one(&pool)
            .await
            .expect("committed canonical rows"),
            LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64
        );

        let pressure_shutdown = CancellationToken::new();
        let pressure_gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
        let held = pressure_gate
            .try_begin_background("test-holder")
            .expect("hold background admission");
        let pressure_control =
            LongTermProjectionWriteControl::background(&pressure_shutdown, &pressure_gate);
        let deferred = upsert_long_term_projection_interval_segments(
            &pool,
            &[projection_interval_segment(
                2_000,
                start_ms,
                start_ms + 1_000,
            )],
            &pressure_control,
        )
        .await
        .expect_err("pressure must defer before opening a write transaction");
        assert!(long_term_projection_write_is_deferred(&deferred));
        drop(held);
    }

    #[tokio::test]
    async fn incremental_projection_defers_before_interval_and_rollup_publication() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        let event = build_long_term_projection_event(&LongTermInvocationRow {
            id: 1,
            invoke_id: Some("atomic-pressure".to_string()),
            occurred_at: "2026-07-26 10:00:00".to_string(),
            status: Some("success".to_string()),
            model: Some("gpt-5".to_string()),
            request_model: None,
            response_model: None,
            reasoning_effort: None,
            upstream_account_id: None,
            upstream_account_kind: None,
            upstream_account_name: None,
            total_tokens: Some(100),
            output_tokens: None,
            cost: None,
            t_total_ms: Some(1_000.0),
            t_req_read_ms: None,
            t_req_parse_ms: None,
            t_upstream_connect_ms: None,
            t_upstream_ttfb_ms: None,
            t_upstream_stream_ms: None,
            error_message: None,
        });
        let runtime = Arc::new(Mutex::new(LongTermProjectionRuntime::default()));
        let shutdown = CancellationToken::new();
        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
        let held = gate
            .try_begin_background("test-holder")
            .expect("hold background admission");
        let pressure_control = LongTermProjectionWriteControl::background(&shutdown, &gate);
        let deferred = apply_long_term_projection_incremental_with_runtime_and_control(
            &pool,
            &runtime,
            LongTermProjectionIncrementalBatch {
                hourly: &event.hourly,
                daily: &event.daily,
                segments: &event.segments,
            },
            event.row_id,
            1,
            &pressure_control,
        )
        .await
        .expect_err("pressure rejects the atomic incremental publication");
        assert!(long_term_projection_write_is_deferred(&deferred));
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_interval_state"
            )
            .fetch_one(&pool)
            .await
            .expect("no canonical interval before publication"),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_usage_daily")
                .fetch_one(&pool)
                .await
                .expect("no rollup before publication"),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE((SELECT cursor_row_id FROM long_term_projection_state WHERE consumer = ?1), 0)",
            )
            .bind(LONG_TERM_PROJECTION_CONSUMER)
            .fetch_one(&pool)
            .await
            .expect("projection cursor"),
            0
        );

        drop(held);
        let control = LongTermProjectionWriteControl::unrestricted();
        apply_long_term_projection_incremental_with_runtime_and_control(
            &pool,
            &runtime,
            LongTermProjectionIncrementalBatch {
                hourly: &event.hourly,
                daily: &event.daily,
                segments: &event.segments,
            },
            event.row_id,
            1,
            &control,
        )
        .await
        .expect("retry publishes the entire event once");
        let daily = sqlx::query_as::<_, (i64, i64)>(
            "SELECT calls, token_total FROM long_term_usage_daily WHERE stats_date = '2026-07-26' AND dimension = 'overall' AND series_key = 'overall'",
        )
        .fetch_one(&pool)
        .await
        .expect("daily projection");
        assert_eq!(daily, (1, 100));
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_interval_state"
            )
            .fetch_one(&pool)
            .await
            .expect("canonical interval after retry"),
            1
        );
        assert_eq!(
            load_long_term_projection_cursor(&pool)
                .await
                .expect("advanced projection cursor"),
            event.row_id
        );
    }

    #[tokio::test]
    async fn incremental_projection_rebuilds_when_a_dirty_marker_arrives_after_ready_read() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        let event = build_long_term_projection_event(&LongTermInvocationRow {
            id: 1,
            invoke_id: Some("dirty-race".to_string()),
            occurred_at: "2026-07-26 10:00:00".to_string(),
            status: Some("success".to_string()),
            model: Some("gpt-5".to_string()),
            request_model: None,
            response_model: None,
            reasoning_effort: None,
            upstream_account_id: None,
            upstream_account_kind: None,
            upstream_account_name: None,
            total_tokens: Some(100),
            output_tokens: None,
            cost: None,
            t_total_ms: Some(1_000.0),
            t_req_read_ms: None,
            t_req_parse_ms: None,
            t_upstream_connect_ms: None,
            t_upstream_ttfb_ms: None,
            t_upstream_stream_ms: None,
            error_message: None,
        });
        let date = "2026-07-26";
        sqlx::query(
            "INSERT INTO long_term_projection_bucket_state (bucket_date, interval_baseline_ready) VALUES (?1, 1)",
        )
        .bind(date)
        .execute(&pool)
        .await
        .expect("seed ready bucket");
        let ready_dates = load_long_term_projection_ready_dates(&pool, &event.bucket_dates)
            .await
            .expect("ready snapshot");
        assert!(event.bucket_dates.is_subset(&ready_dates));

        // A correction can land after the ready read and before the incremental transaction.
        sqlx::query(
            "INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason) VALUES (?1, 'test-race')",
        )
        .bind(date)
        .execute(&pool)
        .await
        .expect("queue correction after ready read");
        let runtime = Arc::new(Mutex::new(LongTermProjectionRuntime::default()));
        let control = LongTermProjectionWriteControl::unrestricted();
        let outcome = apply_long_term_projection_incremental_with_runtime_and_control(
            &pool,
            &runtime,
            LongTermProjectionIncrementalBatch {
                hourly: &event.hourly,
                daily: &event.daily,
                segments: &event.segments,
            },
            event.row_id,
            1,
            &control,
        )
        .await
        .expect("dirty revalidation");
        assert_eq!(
            outcome,
            LongTermProjectionIncrementalOutcome::RebuildRequired
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_interval_state"
            )
            .fetch_one(&pool)
            .await
            .expect("no canonical interval publication"),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_usage_daily")
                .fetch_one(&pool)
                .await
                .expect("no mixed daily publication"),
            0
        );
        assert_eq!(
            load_long_term_projection_cursor(&pool)
                .await
                .expect("unadvanced cursor"),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_dirty_buckets WHERE bucket_date = ?1",
            )
            .bind(date)
            .fetch_one(&pool)
            .await
            .expect("durable dirty marker"),
            1
        );
    }

    #[tokio::test]
    async fn legacy_projection_intervals_migrate_without_losing_the_canonical_interval() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_projection_schema(&pool)
            .await
            .expect("projection schema");
        for (bucket_kind, bucket_key, dimension, series_key) in [
            ("daily", "2026-07-26", "overall", "overall"),
            ("daily", "2026-07-26", "model", "model:gpt-5:high"),
            ("daily", "2026-07-26", "upstream", "account:42"),
            ("hourly", "1785031200", "overall", "overall"),
            ("hourly", "1785031200", "model", "model:gpt-5:high"),
            ("hourly", "1785031200", "upstream", "account:42"),
        ] {
            sqlx::query(
                "INSERT INTO long_term_projection_intervals (bucket_kind, bucket_date, bucket_key, dimension, series_key, invocation_row_id, interval_start_ms, interval_end_ms) VALUES (?1, '2026-07-26', ?2, ?3, ?4, 7, 10, 20)",
            )
            .bind(bucket_kind)
            .bind(bucket_key)
            .bind(dimension)
            .bind(series_key)
            .execute(&pool)
            .await
            .expect("legacy expanded interval");
        }
        let control = LongTermProjectionWriteControl::unrestricted();
        assert!(
            migrate_long_term_projection_legacy_interval_state(&pool, &control)
                .await
                .expect("canonical legacy interval migration")
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_interval_state WHERE invocation_row_id = 7",
            )
            .fetch_one(&pool)
            .await
            .expect("canonical migration state"),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_intervals")
                .fetch_one(&pool)
                .await
                .expect("legacy expansion remains for a later bounded cleanup"),
            6
        );
        assert!(
            migrate_long_term_projection_legacy_interval_state(&pool, &control)
                .await
                .expect("bounded legacy cleanup")
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_intervals")
                .fetch_one(&pool)
                .await
                .expect("legacy cleanup complete"),
            0
        );
    }

    #[tokio::test]
    async fn legacy_projection_interval_migration_compresses_expansion_in_one_cancellable_batch() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_projection_schema(&pool)
            .await
            .expect("projection schema");
        for invocation_row_id in 1..=256 {
            for bucket_kind in ["daily", "hourly"] {
                for (dimension, series_key) in [
                    ("overall", "overall"),
                    ("model", "model:gpt-5:high"),
                    ("upstream", "account:42"),
                ] {
                    sqlx::query(
                        "INSERT INTO long_term_projection_intervals (bucket_kind, bucket_date, bucket_key, dimension, series_key, invocation_row_id, interval_start_ms, interval_end_ms) VALUES (?1, '2026-07-26', ?2, ?3, ?4, ?5, 10, 20)",
                    )
                    .bind(bucket_kind)
                    .bind(format!("{bucket_kind}-{invocation_row_id}"))
                    .bind(dimension)
                    .bind(series_key)
                    .bind(invocation_row_id)
                    .execute(&pool)
                    .await
                    .expect("legacy expanded interval");
                }
            }
        }

        let shutdown = CancellationToken::new();
        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
        let committed_batches = AtomicUsize::new(0);
        let interrupted =
            LongTermProjectionWriteControl::stopping_after(&shutdown, &gate, &committed_batches, 1);
        assert!(
            migrate_long_term_projection_legacy_interval_state(&pool, &interrupted)
                .await
                .expect("one canonical compression batch")
        );
        assert_eq!(committed_batches.load(Ordering::Acquire), 1);
        let interrupted_state = sqlx::query_as::<_, LongTermProjectionIntervalStateRow>(
            "SELECT invocation_row_id, model_series_key, upstream_series_key, interval_start_ms, interval_end_ms FROM long_term_projection_interval_state WHERE invocation_row_id = 9",
        )
        .fetch_one(&pool)
        .await
        .expect("canonical interval after interrupted cleanup");
        assert_eq!(interrupted_state.model_series_key, "model:gpt-5:high");
        assert_eq!(interrupted_state.upstream_series_key, "account:42");
        assert_eq!(interrupted_state.interval_start_ms, 10);
        assert_eq!(interrupted_state.interval_end_ms, 20);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_intervals")
                .fetch_one(&pool)
                .await
                .expect("all expanded rows remain before cleanup"),
            1_536
        );
        assert!(
            long_term_projection_maintenance_needed(&pool, 366)
                .await
                .expect("legacy expansion keeps the next maintenance pass scheduled")
        );
        let error = migrate_long_term_projection_legacy_interval_state(&pool, &interrupted)
            .await
            .expect_err("cancellation stops the next legacy cleanup batch");
        assert!(error.to_string().contains("cancelled"));

        let unrestricted = LongTermProjectionWriteControl::unrestricted();
        for expected_remaining in [1_024_i64, 512, 0] {
            assert!(
                migrate_long_term_projection_legacy_interval_state(&pool, &unrestricted)
                    .await
                    .expect("resume one bounded legacy cleanup batch")
            );
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_intervals")
                    .fetch_one(&pool)
                    .await
                    .expect("remaining expanded legacy rows"),
                expected_remaining
            );
        }
        assert!(
            !migrate_long_term_projection_legacy_interval_state(&pool, &unrestricted)
                .await
                .expect("completed migration has no further work")
        );
        let resumed_state = sqlx::query_as::<_, LongTermProjectionIntervalStateRow>(
            "SELECT invocation_row_id, model_series_key, upstream_series_key, interval_start_ms, interval_end_ms FROM long_term_projection_interval_state WHERE invocation_row_id = 9",
        )
        .fetch_one(&pool)
        .await
        .expect("canonical interval after resumed cleanup");
        assert_eq!(resumed_state.model_series_key, "model:gpt-5:high");
        assert_eq!(resumed_state.upstream_series_key, "account:42");
        assert_eq!(resumed_state.interval_start_ms, 10);
        assert_eq!(resumed_state.interval_end_ms, 20);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_intervals")
                .fetch_one(&pool)
                .await
                .expect("completed legacy cleanup"),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_interval_state"
            )
            .fetch_one(&pool)
            .await
            .expect("one durable interval state per invocation"),
            256
        );
    }

    #[tokio::test]
    async fn interrupted_date_rebuild_keeps_cross_day_interval_for_neighboring_date() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        let first_date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("first date");
        let second_date = first_date.succ_opt().expect("second date");
        let (first_start, first_end) =
            long_term_day_epoch_bounds(first_date).expect("first bounds");
        let (_, second_end) = long_term_day_epoch_bounds(second_date).expect("second bounds");
        let segment =
            projection_interval_segment(11, first_end * 1_000 - 1_000, second_end * 1_000 - 1_000);
        let unrestricted = LongTermProjectionWriteControl::unrestricted();
        upsert_long_term_projection_interval_segments(&pool, &[segment], &unrestricted)
            .await
            .expect("seed cross-day canonical interval");
        let shutdown = CancellationToken::new();
        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
        let committed_batches = AtomicUsize::new(0);
        let cancelling_control =
            LongTermProjectionWriteControl::stopping_after(&shutdown, &gate, &committed_batches, 1);
        let rebuild = LongTermProjectionDateRebuild {
            bucket_date: first_date.to_string(),
            start_epoch: first_start,
            end_epoch: first_end,
            hourly: HashMap::new(),
            daily: HashMap::new(),
            interval_segments: Vec::new(),
            source_row_count: 0,
        };
        commit_long_term_projection_date_rebuilds_with_control(
            &pool,
            &[rebuild],
            None,
            &[],
            false,
            &cancelling_control,
        )
        .await
        .expect_err("rebuild cancellation after the staging batch");
        let neighboring_index = load_long_term_projection_interval_index(
            &pool,
            &HashSet::from([second_date.to_string()]),
        )
        .await
        .expect("neighboring durable interval index");
        assert!(
            neighboring_index
                .values()
                .any(|union| union.duration_ms > 0)
        );
    }

    #[tokio::test]
    async fn daily_backup_claim_keeps_competing_rebuild_from_publishing_partial_rows() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
        let date_text = date.to_string();
        sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls) VALUES (?1, 'model', 'model:last-good', 'last good', 41)",
        )
        .bind(&date_text)
        .execute(&pool)
        .await
        .expect("seed last-good daily row");
        sqlx::query(
            "INSERT INTO long_term_projection_daily_backup_claims (bucket_date, rebuild_token) VALUES (?1, 'owner-one')",
        )
        .bind(&date_text)
        .execute(&pool)
        .await
        .expect("reserve first owner");
        let control = LongTermProjectionWriteControl::unrestricted();

        let error = ensure_long_term_projection_daily_backup_for_date(
            &pool,
            &date_text,
            "owner-two",
            true,
            &control,
        )
        .await
        .expect_err("competing owner cannot replace an uncommitted snapshot");
        assert!(error.to_string().contains("owner-one"));
        let public_rows = load_long_term_daily_rows(&pool, "model", None, &date_text, &date_text)
            .await
            .expect("public live rows while snapshot is reserved");
        assert_eq!(public_rows.len(), 1);
        assert_eq!(public_rows[0].series_key, "model:last-good");
        assert_eq!(public_rows[0].calls, 41);

        ensure_long_term_projection_daily_backup_for_date(
            &pool,
            &date_text,
            "owner-one",
            true,
            &control,
        )
        .await
        .expect("first owner completes a snapshot");
        let active = sqlx::query_scalar::<_, Option<String>>(
            "SELECT active_daily_backup_token FROM long_term_projection_bucket_state WHERE bucket_date = ?1",
        )
        .bind(&date_text)
        .fetch_one(&pool)
        .await
        .expect("active backup owner");
        assert_eq!(active.as_deref(), Some("owner-one"));
        let error = ensure_long_term_projection_daily_backup_for_date(
            &pool,
            &date_text,
            "owner-two",
            true,
            &control,
        )
        .await
        .expect_err("competing owner cannot replace a published snapshot");
        assert!(error.to_string().contains("owner-one"));

        release_long_term_projection_daily_backups(
            &pool,
            &[(date_text.clone(), "owner-one".to_string())],
            &control,
        )
        .await
        .expect("release completed owner");
        let claims = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_projection_daily_backup_claims WHERE bucket_date = ?1",
        )
        .bind(&date_text)
        .fetch_one(&pool)
        .await
        .expect("released claim");
        assert_eq!(claims, 0);
    }

    #[tokio::test]
    async fn initial_marker_blocks_a_stale_baseline_from_publishing_ready() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        sqlx::query("UPDATE long_term_stats_state SET status = ?1, last_error = ?2 WHERE id = ?3")
            .bind(LONG_TERM_STATUS_RUNNING)
            .bind(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR)
            .bind(LONG_TERM_STATE_ID)
            .execute(&pool)
            .await
            .expect("persist incomplete initial marker");
        let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
        let (start_epoch, end_epoch) = long_term_day_epoch_bounds(date).expect("projection bounds");
        let rebuild = LongTermProjectionDateRebuild {
            bucket_date: date.to_string(),
            start_epoch,
            end_epoch,
            hourly: HashMap::new(),
            daily: HashMap::new(),
            interval_segments: Vec::new(),
            source_row_count: 0,
        };
        let control = LongTermProjectionWriteControl::unrestricted();

        let error = commit_long_term_projection_date_rebuilds_with_control(
            &pool,
            &[rebuild],
            Some(321),
            &[],
            true,
            &control,
        )
        .await
        .expect_err("stale P2 baseline must not publish over the initial marker");
        assert!(
            error
                .to_string()
                .contains("incomplete initial materialization")
        );
        let state = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT status, last_error FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("initial state remains retryable");
        assert_eq!(state.0, LONG_TERM_STATUS_RUNNING);
        assert_eq!(
            state.1.as_deref(),
            Some(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR)
        );
        assert_eq!(
            load_long_term_projection_cursor(&pool)
                .await
                .expect("cursor remains before rejected publication"),
            0
        );
    }

    #[tokio::test]
    async fn interrupted_date_rebuild_keeps_publication_and_cursor_uncommitted() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
        let (start_epoch, end_epoch) = long_term_day_epoch_bounds(date).expect("projection bounds");
        let date_text = date.to_string();
        let old = LongTermBucket {
            bucket_start_epoch: start_epoch,
            dimension: "model".to_string(),
            series_key: "model:last-good".to_string(),
            display_name: "last good".to_string(),
            reasoning_effort: String::new(),
            stats_date: Some(date_text.clone()),
            accumulator: LongTermAccumulator {
                calls: 41,
                ..LongTermAccumulator::default()
            },
        };
        let mut transaction = pool.begin().await.expect("seed transaction");
        insert_long_term_daily(&mut transaction, &old)
            .await
            .expect("seed last-good daily row");
        transaction
            .commit()
            .await
            .expect("commit last-good daily row");
        sqlx::query(
            "INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason) VALUES (?1, 'test')",
        )
        .bind(&date_text)
        .execute(&pool)
        .await
        .expect("seed dirty projection date");

        let mut daily = HashMap::new();
        for index in 0..=LONG_TERM_PROJECTION_WRITE_BATCH_ROWS {
            let series_key = format!("model:rebuilt-{index}");
            daily.insert(
                (date_text.clone(), "model".to_string(), series_key.clone()),
                LongTermBucket {
                    bucket_start_epoch: start_epoch,
                    dimension: "model".to_string(),
                    series_key,
                    display_name: "rebuilt".to_string(),
                    reasoning_effort: String::new(),
                    stats_date: Some(date_text.clone()),
                    accumulator: LongTermAccumulator {
                        calls: 1,
                        ..LongTermAccumulator::default()
                    },
                },
            );
        }
        let shutdown = CancellationToken::new();
        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
        let committed_batches = AtomicUsize::new(0);
        let control = LongTermProjectionWriteControl::cancelling_after(
            &shutdown,
            &gate,
            &committed_batches,
            10,
        );
        let rebuild = LongTermProjectionDateRebuild {
            bucket_date: date_text.clone(),
            start_epoch,
            end_epoch,
            hourly: HashMap::new(),
            daily,
            interval_segments: Vec::new(),
            source_row_count: 0,
        };
        commit_long_term_projection_date_rebuilds_with_control(
            &pool,
            std::slice::from_ref(&rebuild),
            Some(321),
            &[LongTermProjectionDirtyBucket {
                bucket_date: date_text.clone(),
                generation: 1,
            }],
            false,
            &control,
        )
        .await
        .expect_err("cancellation before atomic publication");
        assert!(shutdown.is_cancelled());
        assert_eq!(committed_batches.load(Ordering::Acquire), 10);
        assert_eq!(
            load_long_term_projection_cursor(&pool)
                .await
                .expect("cursor remains before publication"),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_dirty_buckets WHERE bucket_date = ?1",
            )
            .bind(&date_text)
            .fetch_one(&pool)
            .await
            .expect("dirty date after interrupted handoff"),
            1
        );
        assert!(
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT active_daily_backup_token FROM long_term_projection_bucket_state WHERE bucket_date = ?1",
            )
            .bind(&date_text)
            .fetch_one(&pool)
            .await
            .expect("active last-good backup")
            .is_some()
        );
        let public_rows = load_long_term_daily_rows(&pool, "model", None, &date_text, &date_text)
            .await
            .expect("public last-good daily rows");
        assert_eq!(public_rows.len(), 1);
        assert_eq!(public_rows[0].series_key, "model:last-good");
        assert_eq!(public_rows[0].calls, 41);

        let unrestricted = LongTermProjectionWriteControl::unrestricted();
        commit_long_term_projection_date_rebuilds_with_control(
            &pool,
            std::slice::from_ref(&rebuild),
            Some(321),
            &[LongTermProjectionDirtyBucket {
                bucket_date: date_text.clone(),
                generation: 1,
            }],
            false,
            &unrestricted,
        )
        .await
        .expect("resume atomically interrupted rebuild");
        assert_eq!(
            load_long_term_projection_cursor(&pool)
                .await
                .expect("cursor after publication"),
            321
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_dirty_buckets WHERE bucket_date = ?1",
            )
            .bind(&date_text)
            .fetch_one(&pool)
            .await
            .expect("cleared dirty date"),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT active_daily_backup_token FROM long_term_projection_bucket_state WHERE bucket_date = ?1",
            )
            .bind(&date_text)
            .fetch_one(&pool)
            .await
            .expect("released backup pointer"),
            None
        );
        let published_rows =
            load_long_term_daily_rows(&pool, "model", None, &date_text, &date_text)
                .await
                .expect("public rebuilt daily rows");
        assert_eq!(
            published_rows.len(),
            LONG_TERM_PROJECTION_WRITE_BATCH_ROWS + 1
        );
        assert!(
            published_rows
                .iter()
                .all(|row| row.series_key.starts_with("model:rebuilt-"))
        );
    }

    #[tokio::test]
    async fn chunked_date_rebuild_keeps_empty_state_until_final_publication() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
            .bind(LONG_TERM_STATUS_EMPTY)
            .bind(LONG_TERM_STATE_ID)
            .execute(&pool)
            .await
            .expect("empty projection state");

        let first_date = NaiveDate::from_ymd_opt(2025, 1, 1).expect("first projection date");
        let first_date_text = first_date.to_string();
        sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls) VALUES (?1, 'model', 'model:last-good', 'last good', 41)",
        )
        .bind(&first_date_text)
        .execute(&pool)
        .await
        .expect("seed last-good first chunk row");
        let mut rebuilds = Vec::with_capacity(LONG_TERM_PROJECTION_REBUILD_PUBLICATION_DATES + 1);
        let mut dirty = Vec::with_capacity(LONG_TERM_PROJECTION_REBUILD_PUBLICATION_DATES + 1);
        for offset in 0..=LONG_TERM_PROJECTION_REBUILD_PUBLICATION_DATES {
            let date = first_date
                .checked_add_signed(ChronoDuration::days(offset as i64))
                .expect("projection date");
            let date_text = date.to_string();
            let (start_epoch, end_epoch) =
                long_term_day_epoch_bounds(date).expect("projection bounds");
            let mut daily = HashMap::new();
            if offset == 0 {
                daily.insert(
                    (
                        date_text.clone(),
                        "model".to_string(),
                        "model:chunked".to_string(),
                    ),
                    LongTermBucket {
                        bucket_start_epoch: start_epoch,
                        dimension: "model".to_string(),
                        series_key: "model:chunked".to_string(),
                        display_name: "chunked".to_string(),
                        reasoning_effort: String::new(),
                        stats_date: Some(date_text.clone()),
                        accumulator: LongTermAccumulator {
                            calls: 1,
                            ..LongTermAccumulator::default()
                        },
                    },
                );
            }
            sqlx::query(
                "INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason) VALUES (?1, 'chunked_test')",
            )
            .bind(&date_text)
            .execute(&pool)
            .await
            .expect("dirty projection date");
            rebuilds.push(LongTermProjectionDateRebuild {
                bucket_date: date_text.clone(),
                start_epoch,
                end_epoch,
                hourly: HashMap::new(),
                daily,
                interval_segments: Vec::new(),
                source_row_count: 0,
            });
            dirty.push(LongTermProjectionDirtyBucket {
                bucket_date: date_text,
                generation: 1,
            });
        }

        let shutdown = CancellationToken::new();
        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
        let interrupted =
            LongTermProjectionWriteControl::stopping_after_rebuild_chunk(&shutdown, &gate);
        commit_long_term_projection_date_rebuilds_with_control(
            &pool,
            &rebuilds,
            Some(321),
            &dirty,
            false,
            &interrupted,
        )
        .await
        .expect_err("second chunk must not start after cancellation");

        let status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("state before final publication");
        assert_eq!(status, LONG_TERM_STATUS_EMPTY);
        assert_eq!(
            load_long_term_projection_cursor(&pool)
                .await
                .expect("cursor remains before final publication"),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_dirty_buckets",
            )
            .fetch_one(&pool)
            .await
            .expect("all dirty dates remain before publication"),
            (LONG_TERM_PROJECTION_REBUILD_PUBLICATION_DATES + 1) as i64
        );
        let visible_rows =
            load_long_term_daily_rows(&pool, "model", None, &first_date_text, &first_date_text)
                .await
                .expect("staged chunk keeps last-good public row");
        assert_eq!(visible_rows.len(), 1);
        assert_eq!(visible_rows[0].series_key, "model:last-good");
        assert_eq!(visible_rows[0].calls, 41);

        let unrestricted = LongTermProjectionWriteControl::unrestricted();
        commit_long_term_projection_date_rebuilds_with_control(
            &pool,
            &rebuilds,
            Some(321),
            &dirty,
            false,
            &unrestricted,
        )
        .await
        .expect("resume final publication");
        let status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("state after final publication");
        assert_eq!(status, LONG_TERM_STATUS_READY);
        assert_eq!(
            load_long_term_projection_cursor(&pool)
                .await
                .expect("cursor after final publication"),
            321
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_dirty_buckets",
            )
            .fetch_one(&pool)
            .await
            .expect("cleared dirty chunks"),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_date_publications",
            )
            .fetch_one(&pool)
            .await
            .expect("completed publication metadata is pruned"),
            0
        );
        let published_rows =
            load_long_term_daily_rows(&pool, "model", None, &first_date_text, &first_date_text)
                .await
                .expect("published first chunk row");
        assert_eq!(published_rows.len(), 1);
        assert_eq!(published_rows[0].series_key, "model:chunked");
    }

    #[tokio::test]
    async fn projection_date_rebuild_standard_timestamp_branch_uses_occurred_at_seek() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        create_long_term_test_invocations(&pool).await;
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        sqlx::query(
            "CREATE INDEX idx_codex_invocations_occurred_at ON codex_invocations (occurred_at)",
        )
        .execute(&pool)
        .await
        .expect("occurred_at index");
        let query =
            long_term_projection_canonical_query("SELECT inv.id FROM codex_invocations inv");
        let plan =
            sqlx::query_as::<_, (i64, i64, i64, String)>(&format!("EXPLAIN QUERY PLAN {query}"))
                .bind("2026-07-26 00:00:00")
                .bind("2026-07-27 00:00:00")
                .fetch_all(&pool)
                .await
                .expect("query plan");
        assert!(plan.iter().any(|(_, _, _, detail)| {
            detail.contains("idx_codex_invocations_occurred_at")
                && detail.contains("occurred_at>? AND occurred_at<?")
        }));
    }

    #[tokio::test]
    async fn projection_date_rebuild_skips_rfc3339_fallback_for_canonical_live_rows() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        create_long_term_test_invocations(&pool).await;
        ensure_long_term_projection_source_indexes(&pool)
            .await
            .expect("projection source indexes");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (1, 'canonical', '2026-07-26 12:00:00', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("canonical invocation");

        let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
        let start = Shanghai
            .from_local_datetime(&date.and_hms_opt(0, 0, 0).expect("day start"))
            .single()
            .expect("Shanghai day start");
        let end = Shanghai
            .from_local_datetime(
                &date
                    .succ_opt()
                    .expect("next date")
                    .and_hms_opt(0, 0, 0)
                    .expect("day end"),
            )
            .single()
            .expect("Shanghai day end");
        assert!(
            load_long_term_projection_live_rfc3339_compatibility(&pool)
                .await
                .expect("canonical compatibility gate")
                .is_none()
        );
        let control = LongTermProjectionWriteControl::unrestricted();
        let canonical_rows =
            load_long_term_projection_rows_for_date(&pool, date, start, end, &control)
                .await
                .expect("canonical projection rows");
        assert_eq!(
            canonical_rows
                .iter()
                .filter_map(|row| row.invoke_id.as_deref())
                .collect::<HashSet<_>>(),
            HashSet::from(["canonical"])
        );

        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (2, 'rfc3339', '2026-07-25T16:00:00Z', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 invocation");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (3, 'rfc3339-negative-offset', '2026-07-25T02:00:01-14:00', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 negative-offset invocation");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (4, 'rfc3339-historical', '2020-01-01T00:00:00Z', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("historical RFC3339 invocation");
        let rfc3339_compatibility = load_long_term_projection_live_rfc3339_compatibility(&pool)
            .await
            .expect("updated RFC3339 compatibility gate")
            .expect("RFC3339 compatibility metadata");
        let (rfc3339_lower, rfc3339_upper) =
            long_term_rfc3339_text_bounds(start, end, &rfc3339_compatibility);
        assert_eq!(rfc3339_lower, "2026-07-25T01:59:59");
        let query =
            long_term_projection_live_rfc3339_query("SELECT inv.id FROM codex_invocations inv");
        let plan =
            sqlx::query_as::<_, (i64, i64, i64, String)>(&format!("EXPLAIN QUERY PLAN {query}"))
                .bind(&rfc3339_lower)
                .bind(&rfc3339_upper)
                .bind(start.timestamp())
                .bind(end.timestamp())
                .fetch_all(&pool)
                .await
                .expect("RFC3339 range query plan");
        assert!(plan.iter().any(|(_, _, _, detail)| {
            detail.contains("idx_codex_invocations_long_term_projection_rfc3339_occurred_at")
                && detail.contains("occurred_at>? AND occurred_at<?")
        }));
        let mixed_rows = load_long_term_projection_rows_for_date(&pool, date, start, end, &control)
            .await
            .expect("mixed projection rows");
        assert_eq!(
            mixed_rows
                .iter()
                .filter_map(|row| row.invoke_id.as_deref())
                .collect::<HashSet<_>>(),
            HashSet::from(["canonical", "rfc3339", "rfc3339-negative-offset"])
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
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, model, payload, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens, cost, t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms) VALUES (1, 'test-invoke-1', ?1, 'success', 'gpt-5', '{\"reasoningEffort\":\"high\"}', 8, 4, 0, 0, 12, 0.2, 100, 10, 5, 5, 20, 80)",
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
    async fn initial_full_rebuild_publishes_a_complete_source_snapshot_without_hourly_proof() {
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
            .expect("publish a complete initial source snapshot without canonical hourly proof");

        let materialized_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("count materialized daily rollups");
        let queued_repairs = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_stats_repair_queue WHERE stats_date = ?1",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("count deferred repairs");
        let state = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT status, statistics_start_date FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("load initial full rebuild state");

        assert_eq!(materialized_rows, 1);
        assert_eq!(queued_repairs, 0);
        assert_eq!(state.0, LONG_TERM_STATUS_READY);
        let expected_start = date.to_string();
        assert_eq!(state.1.as_deref(), Some(expected_start.as_str()));
    }

    #[tokio::test]
    async fn initial_full_rebuild_recovers_a_queued_repair_from_a_complete_source_snapshot() {
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
            "INSERT INTO long_term_stats_repair_queue (stats_date, expected_calls, expected_token_total, expected_cost_total, observed_calls, observed_token_total, observed_cost_total, last_error) VALUES (?1, 2, 200, 0.2, 1, 100, 0.1, 'untrusted hourly proof')",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("queue repair without canonical hourly proof");

        refresh_long_term_stats(&pool, 400)
            .await
            .expect("complete initial source snapshot resolves the queued repair");

        let materialized_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("count materialized daily rollups");
        let queued_repairs = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_stats_repair_queue WHERE stats_date = ?1",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("count resolved repairs");
        let status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("load completed initial materialization state");

        assert_eq!(materialized_rows, 1);
        assert_eq!(queued_repairs, 0);
        assert_eq!(status, LONG_TERM_STATUS_READY);
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
            None,
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
            None,
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

    #[tokio::test]
    async fn long_term_refresh_cancels_before_a_second_sqlite_lock_retry() {
        let shutdown = CancellationToken::new();
        let operation_shutdown = shutdown.clone();
        let attempts = Arc::new(AtomicUsize::new(0));
        let operation_attempts = Arc::clone(&attempts);
        let error = run_long_term_refresh_with_retry_delays(
            Some(&shutdown),
            move || {
                let operation_attempts = Arc::clone(&operation_attempts);
                let operation_shutdown = operation_shutdown.clone();
                async move {
                    operation_attempts.fetch_add(1, Ordering::SeqCst);
                    operation_shutdown.cancel();
                    Err::<(), _>(anyhow::anyhow!("database is locked"))
                }
            },
            &[Duration::from_secs(1)],
        )
        .await
        .expect_err("shutdown stops the initial refresh before another lock retry");
        assert!(error.to_string().contains("cancelled"));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn projection_flush_retries_a_transient_sqlite_lock() {
        let shutdown = CancellationToken::new();
        let attempts = Arc::new(AtomicUsize::new(0));
        let operation_attempts = Arc::clone(&attempts);
        let outcome = run_long_term_projection_flush_with_retry_delays(
            &shutdown,
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
        .expect("transient lock retries before deferring a repair");
        assert_eq!(outcome, 3);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn projection_flush_cancels_during_sqlite_lock_retry() {
        let shutdown = CancellationToken::new();
        let operation_shutdown = shutdown.clone();
        let attempts = Arc::new(AtomicUsize::new(0));
        let operation_attempts = Arc::clone(&attempts);
        let error = run_long_term_projection_flush_with_retry_delays(
            &shutdown,
            move || {
                let operation_attempts = Arc::clone(&operation_attempts);
                let operation_shutdown = operation_shutdown.clone();
                async move {
                    operation_attempts.fetch_add(1, Ordering::SeqCst);
                    operation_shutdown.cancel();
                    Err::<(), _>(anyhow::anyhow!("database is locked"))
                }
            },
            &[Duration::ZERO, Duration::ZERO, Duration::ZERO],
        )
        .await
        .expect_err("shutdown stops the lock retry without another write attempt");
        assert!(long_term_projection_write_is_deferred(&error));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn projection_date_rebuild_filters_rfc3339_rows_by_shanghai_epoch_bounds() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        create_long_term_test_invocations(&pool).await;
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (1, 'utc-boundary', '2026-07-25T16:30:00Z', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("boundary invocation");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (2, 'local-boundary', '2026-07-26 00:30:00', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("local boundary invocation");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens, t_total_ms) VALUES (3, 'rfc3339-crossing', '2026-07-25T15:59:59.500Z', 'success', 100, 600)",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 crossing invocation");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens, t_total_ms) VALUES (4, 'legacy-crossing', '2026-07-25 23:59:59', 'success', 100, 2000)",
        )
        .execute(&pool)
        .await
        .expect("legacy crossing invocation");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens, t_total_ms) VALUES (5, 'rfc3339-outside', '2026-07-25T15:59:57Z', 'success', 100, 1000)",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 outside invocation");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (6, 'rfc3339-day-start', '2026-07-25T16:00:00Z', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 day-start invocation");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (7, 'rfc3339-next-day-start', '2026-07-26T16:00:00Z', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 next-day-start invocation");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (8, 'rfc3339-before-day-start', '2026-07-25T15:59:59.9999Z', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 sub-millisecond pre-start invocation");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (9, 'rfc3339-before-next-day-start', '2026-07-26T15:59:59.9999Z', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 sub-millisecond pre-end invocation");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (10, 'rfc3339-submicro-before-day-start', '2026-07-25T15:59:59.9999999Z', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 sub-microsecond pre-start invocation");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (11, 'rfc3339-submicro-before-next-day-start', '2026-07-26T15:59:59.9999999Z', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 sub-microsecond pre-end invocation");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens, t_total_ms) VALUES (12, 'rfc3339-nanos-crossing', '2026-07-25T15:59:59.9999999Z', 'success', 100, 0.001)",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 nanosecond crossing invocation");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (13, 'rfc3339-high-precision-before-start', '2026-07-25T15:59:59.99999999999999999999Z', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("high-precision RFC3339 pre-start invocation");
        let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("fixed date");
        let start = Shanghai
            .from_local_datetime(&date.and_hms_opt(0, 0, 0).expect("day start"))
            .single()
            .expect("Shanghai day start");
        let end = Shanghai
            .from_local_datetime(
                &date
                    .succ_opt()
                    .expect("next date")
                    .and_hms_opt(0, 0, 0)
                    .expect("day end"),
            )
            .single()
            .expect("Shanghai day end");
        let crossing_query =
            long_term_projection_crossing_text_query("SELECT inv.id FROM codex_invocations inv");
        let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(&format!(
            "EXPLAIN QUERY PLAN {crossing_query}"
        ))
        .bind(start.format("%Y-%m-%d %H:%M:%S").to_string())
        .fetch_all(&pool)
        .await
        .expect("crossing query plan");
        assert!(plan.iter().any(|(_, _, _, detail)| {
            detail.contains("idx_codex_invocations_long_term_projection_text_end")
        }));

        let control = LongTermProjectionWriteControl::unrestricted();
        let rows = load_long_term_projection_rows_for_date(&pool, date, start, end, &control)
            .await
            .expect("projection rows");

        assert_eq!(rows.len(), 8);
        let ids = rows
            .iter()
            .filter_map(|row| row.invoke_id.as_deref())
            .collect::<HashSet<_>>();
        assert_eq!(
            ids,
            HashSet::from([
                "utc-boundary",
                "local-boundary",
                "rfc3339-crossing",
                "legacy-crossing",
                "rfc3339-day-start",
                "rfc3339-before-next-day-start",
                "rfc3339-submicro-before-next-day-start",
                "rfc3339-nanos-crossing",
            ])
        );

        let rebuild = build_long_term_projection_date_rebuild(&pool, "2026-07-26", &control)
            .await
            .expect("projection rebuild retains the nanosecond crossing");
        assert!(
            rebuild
                .daily
                .keys()
                .any(|(bucket_date, dimension, series_key)| {
                    bucket_date == "2026-07-26" && dimension == "overall" && series_key == "overall"
                }),
            "a selected positive crossing interval must materialize the target date"
        );
        assert!(rebuild.interval_segments.iter().any(|segment| {
            segment.invocation_row_id == 12
                && long_term_projection_interval_dates(segment).contains("2026-07-26")
        }));
    }

    #[tokio::test]
    async fn projection_rebuild_preserves_a_newer_dirty_generation() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        let dates = vec!["2026-07-26".to_string()];
        queue_long_term_projection_repairs(&pool, &dates, "first")
            .await
            .expect("first repair");
        let stale_marker = load_long_term_projection_dirty_buckets(&pool, &dates)
            .await
            .expect("stale marker");
        queue_long_term_projection_repairs(&pool, &dates, "raced_correction")
            .await
            .expect("raced repair");

        commit_long_term_projection_date_rebuilds(&pool, &[], None, &stale_marker, false)
            .await
            .expect("commit stale rebuild");

        let generation = sqlx::query_scalar::<_, i64>(
            "SELECT generation FROM long_term_projection_dirty_buckets WHERE bucket_date = ?1",
        )
        .bind(&dates[0])
        .fetch_one(&pool)
        .await
        .expect("newer marker remains");
        assert_eq!(generation, 2);
    }

    #[tokio::test]
    async fn invocation_correction_marks_rfc3339_fractional_cross_day_buckets() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, source TEXT, status TEXT, occurred_at TEXT, model TEXT, payload TEXT, input_tokens INTEGER, output_tokens INTEGER, cache_input_tokens INTEGER, reasoning_tokens INTEGER, total_tokens INTEGER, cost REAL, t_total_ms REAL, t_req_read_ms REAL, t_req_parse_ms REAL, t_upstream_connect_ms REAL, t_upstream_ttfb_ms REAL, t_upstream_stream_ms REAL, error_message TEXT, failure_kind TEXT)",
        )
        .execute(&pool)
        .await
        .expect("invocation schema");
        ensure_long_term_projection_schema(&pool)
            .await
            .expect("projection schema");
        ensure_long_term_projection_correction_trigger(&pool)
            .await
            .expect("correction trigger");
        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, model, t_total_ms) VALUES (1, '2026-07-25T15:59:59.500Z', 'success', 'before', 600)",
        )
        .execute(&pool)
        .await
        .expect("fractional RFC3339 invocation");

        sqlx::query("UPDATE codex_invocations SET model = 'after' WHERE id = 1")
            .execute(&pool)
            .await
            .expect("historical correction");
        let dirty_dates = sqlx::query_scalar::<_, String>(
            "SELECT bucket_date FROM long_term_projection_dirty_buckets ORDER BY bucket_date",
        )
        .fetch_all(&pool)
        .await
        .expect("dirty correction dates");
        assert_eq!(dirty_dates, vec!["2026-07-25", "2026-07-26"]);

        sqlx::query("DELETE FROM long_term_projection_dirty_buckets")
            .execute(&pool)
            .await
            .expect("clear fractional correction markers");
        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, model) VALUES (2, '2026-07-25T16:00:00Z', 'success', 'before')",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 day-start invocation");
        sqlx::query("UPDATE codex_invocations SET model = 'after' WHERE id = 2")
            .execute(&pool)
            .await
            .expect("RFC3339 day-start correction");
        let dirty_dates = sqlx::query_scalar::<_, String>(
            "SELECT bucket_date FROM long_term_projection_dirty_buckets ORDER BY bucket_date",
        )
        .fetch_all(&pool)
        .await
        .expect("day-start correction dates");
        assert_eq!(dirty_dates, vec!["2026-07-26"]);

        sqlx::query("DELETE FROM long_term_projection_dirty_buckets")
            .execute(&pool)
            .await
            .expect("clear day-start correction markers");
        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, model) VALUES (3, '2026-07-25T15:59:59.9999Z', 'success', 'before')",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 sub-millisecond pre-start invocation");
        sqlx::query("UPDATE codex_invocations SET model = 'after' WHERE id = 3")
            .execute(&pool)
            .await
            .expect("RFC3339 sub-millisecond pre-start correction");
        let dirty_dates = sqlx::query_scalar::<_, String>(
            "SELECT bucket_date FROM long_term_projection_dirty_buckets ORDER BY bucket_date",
        )
        .fetch_all(&pool)
        .await
        .expect("sub-millisecond pre-start correction dates");
        assert_eq!(dirty_dates, vec!["2026-07-25"]);

        sqlx::query("DELETE FROM long_term_projection_dirty_buckets")
            .execute(&pool)
            .await
            .expect("clear cross-day correction markers");
        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, model, t_total_ms) VALUES (4, '2026-07-25T15:59:59.9999Z', 'success', 'before', 1)",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 sub-millisecond cross-day invocation");
        sqlx::query("UPDATE codex_invocations SET model = 'after' WHERE id = 4")
            .execute(&pool)
            .await
            .expect("RFC3339 sub-millisecond cross-day correction");
        let dirty_dates = sqlx::query_scalar::<_, String>(
            "SELECT bucket_date FROM long_term_projection_dirty_buckets ORDER BY bucket_date",
        )
        .fetch_all(&pool)
        .await
        .expect("cross-day correction dates");
        assert_eq!(dirty_dates, vec!["2026-07-25", "2026-07-26"]);

        sqlx::query("DELETE FROM long_term_projection_dirty_buckets")
            .execute(&pool)
            .await
            .expect("clear sub-microsecond correction markers");
        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, model) VALUES (5, '2026-07-25T15:59:59.9999999Z', 'success', 'before')",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 sub-microsecond pre-start invocation");
        sqlx::query("UPDATE codex_invocations SET model = 'after' WHERE id = 5")
            .execute(&pool)
            .await
            .expect("RFC3339 sub-microsecond pre-start correction");
        let dirty_dates = sqlx::query_scalar::<_, String>(
            "SELECT bucket_date FROM long_term_projection_dirty_buckets ORDER BY bucket_date",
        )
        .fetch_all(&pool)
        .await
        .expect("sub-microsecond pre-start correction dates");
        assert_eq!(dirty_dates, vec!["2026-07-25"]);

        sqlx::query("DELETE FROM long_term_projection_dirty_buckets")
            .execute(&pool)
            .await
            .expect("clear high-precision correction markers");
        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, model) VALUES (6, '2026-07-25T15:59:59.99999999999999999999Z', 'success', 'before')",
        )
        .execute(&pool)
        .await
        .expect("high-precision pre-start invocation");
        sqlx::query("UPDATE codex_invocations SET model = 'after' WHERE id = 6")
            .execute(&pool)
            .await
            .expect("high-precision pre-start correction");
        let dirty_dates = sqlx::query_scalar::<_, String>(
            "SELECT bucket_date FROM long_term_projection_dirty_buckets ORDER BY bucket_date",
        )
        .fetch_all(&pool)
        .await
        .expect("high-precision correction dates");
        assert_eq!(dirty_dates, vec!["2026-07-25"]);
    }

    #[tokio::test]
    async fn archive_compatibility_cache_is_checksum_and_opened_bytes_scoped() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_projection_schema(&pool)
            .await
            .expect("projection schema");
        let control = LongTermProjectionWriteControl::unrestricted();
        let original = LongTermArchiveCompatibility {
            has_legacy_crossing: true,
            legacy_max_duration_ms: Some(2_000.0),
            legacy_min_occurred_at: Some("2026-07-25 23:59:59".to_string()),
            has_rfc3339: false,
            rfc3339_max_duration_ms: None,
            rfc3339_min_occurred_at: None,
        };
        let archive_path = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-long-term-archive-fingerprint-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        std::fs::write(&archive_path, b"canonical")
            .expect("write original same-sized archive bytes");
        let original_metadata =
            std::fs::metadata(&archive_path).expect("original archive metadata");
        let original_mtime = original_metadata
            .modified()
            .expect("original archive mtime");
        let original_fingerprint =
            long_term_archive_file_fingerprint(archive_path.to_str().expect("UTF-8 archive path"))
                .expect("fingerprint original archive bytes");
        let archive_file_path = archive_path.to_string_lossy().to_string();
        persist_long_term_archive_compatibility(
            &pool,
            &archive_file_path,
            "archive-sha-one",
            &original_fingerprint,
            original.clone(),
            &control,
        )
        .await
        .expect("persist archive capability");
        assert_eq!(
            load_long_term_archive_compatibility(
                &pool,
                &archive_file_path,
                "archive-sha-one",
                &original_fingerprint,
            )
            .await
            .expect("load matching capability"),
            Some(original)
        );
        assert_eq!(
            load_long_term_archive_compatibility(
                &pool,
                &archive_file_path,
                "archive-sha-two",
                &original_fingerprint,
            )
            .await
            .expect("reject stale capability"),
            None
        );
        std::fs::write(&archive_path, b"rfc-3333!")
            .expect("write replacement same-sized archive bytes");
        filetime::set_file_mtime(
            &archive_path,
            filetime::FileTime::from_system_time(original_mtime),
        )
        .expect("restore replacement archive mtime");
        assert_eq!(
            std::fs::metadata(&archive_path)
                .expect("replacement archive metadata")
                .len(),
            original_metadata.len()
        );
        let replacement_fingerprint =
            long_term_archive_file_fingerprint(archive_path.to_str().expect("UTF-8 archive path"))
                .expect("fingerprint replacement archive bytes");
        assert_ne!(replacement_fingerprint, original_fingerprint);
        assert_eq!(
            load_long_term_archive_compatibility(
                &pool,
                &archive_file_path,
                "archive-sha-one",
                &replacement_fingerprint,
            )
            .await
            .expect("reject replaced archive capability"),
            None
        );
        std::fs::remove_file(&archive_path).expect("remove temporary archive file");
    }

    #[tokio::test]
    async fn archive_pool_fingerprint_uses_the_opened_database_bytes() {
        let unique = format!(
            "{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        );
        let archive_path = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-long-term-fingerprint-source-{unique}.sqlite.gz"
        ));
        let opened_path = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-long-term-fingerprint-opened-{unique}.sqlite"
        ));
        std::fs::write(
            &archive_path,
            b"archive bytes that are not the opened database",
        )
        .expect("write archive source bytes");
        fs::File::create(&opened_path).expect("create opened archive database");
        let options = format!("sqlite://{}", opened_path.to_string_lossy())
            .parse::<SqliteConnectOptions>()
            .expect("parse opened archive database URL")
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("open materialized archive database");
        sqlx::query("CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY)")
            .execute(&pool)
            .await
            .expect("write materialized archive database");

        let opened_fingerprint = long_term_archive_pool_fingerprint(&pool)
            .await
            .expect("fingerprint opened archive database");
        assert_eq!(
            opened_fingerprint,
            long_term_archive_file_fingerprint(opened_path.to_str().expect("UTF-8 opened path"))
                .expect("fingerprint opened archive path")
        );
        assert_ne!(
            opened_fingerprint,
            long_term_archive_file_fingerprint(archive_path.to_str().expect("UTF-8 archive path"))
                .expect("fingerprint archive source path")
        );

        pool.close().await;
        std::fs::remove_file(&archive_path).expect("remove archive source file");
        std::fs::remove_file(&opened_path).expect("remove opened archive database");
    }

    #[tokio::test]
    async fn archive_compatibility_inspection_checks_cancellation_between_bounded_batches() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, occurred_at TEXT NOT NULL, status TEXT, t_total_ms REAL)",
        )
        .execute(&pool)
        .await
        .expect("archive invocation schema");
        sqlx::query(
            r#"
            WITH RECURSIVE source(id) AS (
                VALUES(1)
                UNION ALL
                SELECT id + 1 FROM source WHERE id < 1025
            )
            INSERT INTO codex_invocations (id, occurred_at, status, t_total_ms)
            SELECT
                id,
                printf('2026-07-%02d 00:00:00', (id % 28) + 1),
                CASE WHEN id = 1025 THEN 'success' ELSE 'running' END,
                1
            FROM source
            "#,
        )
        .execute(&pool)
        .await
        .expect("archive source rows");
        let query = long_term_archive_invocation_query_for_range(&pool)
            .await
            .expect("archive range queries");
        let shutdown = CancellationToken::new();
        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
        let interrupted =
            LongTermProjectionWriteControl::stopping_after_archive_compatibility_batch(
                &shutdown, &gate,
            );
        let error = inspect_long_term_archive_compatibility(&pool, &query.parts, &interrupted)
            .await
            .expect_err("cancellation stops before the second bounded scan");
        assert!(long_term_projection_write_is_deferred(&error));

        let recovered_control = LongTermProjectionWriteControl::unrestricted();
        let recovered =
            inspect_long_term_archive_compatibility(&pool, &query.parts, &recovered_control)
                .await
                .expect("a later attempt rescans and recovers compatibility");
        assert!(recovered.has_legacy_crossing);
        assert_eq!(recovered.legacy_max_duration_ms, Some(1.0));
    }

    #[tokio::test]
    async fn archive_compatibility_inspection_includes_the_minimum_rowid() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, occurred_at TEXT NOT NULL, status TEXT, t_total_ms REAL)",
        )
        .execute(&pool)
        .await
        .expect("archive invocation schema");
        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status) VALUES (?1, '2026-07-25T02:00:01-14:00', 'success')",
        )
        .bind(i64::MIN)
        .execute(&pool)
        .await
        .expect("minimum rowid archive invocation");
        let query = long_term_archive_invocation_query_for_range(&pool)
            .await
            .expect("archive range queries");
        let control = LongTermProjectionWriteControl::unrestricted();
        let compatibility = inspect_long_term_archive_compatibility(&pool, &query.parts, &control)
            .await
            .expect("archive compatibility probe");
        assert!(compatibility.has_rfc3339);
        assert_eq!(
            compatibility.rfc3339_min_occurred_at.as_deref(),
            Some("2026-07-25T02:00:01-14:00")
        );
    }

    #[tokio::test]
    async fn archive_range_query_keeps_fractional_rfc3339_cross_day_rows() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, occurred_at TEXT NOT NULL, status TEXT, t_total_ms REAL)",
        )
        .execute(&pool)
        .await
        .expect("archive invocation schema");
        sqlx::query(
            "CREATE INDEX idx_archive_invocations_occurred_at ON codex_invocations (occurred_at)",
        )
        .execute(&pool)
        .await
        .expect("archive occurred_at index");
        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, t_total_ms) VALUES (1, '2026-07-25T15:59:59.500Z', 'success', 600)",
        )
        .execute(&pool)
        .await
        .expect("archive fractional RFC3339 row");
        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status) VALUES (2, '2026-07-25T16:00:00Z', 'success')",
        )
        .execute(&pool)
        .await
        .expect("archive RFC3339 day-start row");
        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status) VALUES (3, '2026-07-26T16:00:00Z', 'success')",
        )
        .execute(&pool)
        .await
        .expect("archive RFC3339 next-day-start row");
        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status) VALUES (4, '2026-07-25T15:59:59.9999Z', 'success')",
        )
        .execute(&pool)
        .await
        .expect("archive RFC3339 sub-millisecond pre-start row");
        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status) VALUES (5, '2026-07-26T15:59:59.9999Z', 'success')",
        )
        .execute(&pool)
        .await
        .expect("archive RFC3339 sub-millisecond pre-end row");
        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status) VALUES (6, '2026-07-25T15:59:59.9999999Z', 'success')",
        )
        .execute(&pool)
        .await
        .expect("archive RFC3339 sub-microsecond pre-start row");
        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status) VALUES (7, '2026-07-26T15:59:59.9999999Z', 'success')",
        )
        .execute(&pool)
        .await
        .expect("archive RFC3339 sub-microsecond pre-end row");
        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, t_total_ms) VALUES (8, '2026-07-25T15:59:59.9999999Z', 'success', 0.001)",
        )
        .execute(&pool)
        .await
        .expect("archive RFC3339 nanosecond crossing row");
        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status) VALUES (9, '2026-07-25T15:59:59.99999999999999999999Z', 'success')",
        )
        .execute(&pool)
        .await
        .expect("archive high-precision RFC3339 pre-start row");
        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, t_total_ms) VALUES (10, '2026-07-25T02:00:01-14:00', 'success', 2000)",
        )
        .execute(&pool)
        .await
        .expect("archive RFC3339 negative-offset row");
        let queries = long_term_archive_invocation_query_for_range(&pool)
            .await
            .expect("archive range queries");
        let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
        let start = Shanghai
            .from_local_datetime(&date.and_hms_opt(0, 0, 0).expect("day start"))
            .single()
            .expect("Shanghai day start");
        let end = Shanghai
            .from_local_datetime(
                &date
                    .succ_opt()
                    .expect("next date")
                    .and_hms_opt(0, 0, 0)
                    .expect("next day start"),
            )
            .single()
            .expect("Shanghai next day start");
        let start_text = start.format("%Y-%m-%d %H:%M:%S").to_string();
        let end_text = end.format("%Y-%m-%d %H:%M:%S").to_string();
        let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(&format!(
            "EXPLAIN QUERY PLAN {}",
            queries.canonical
        ))
        .bind(&start_text)
        .bind(&end_text)
        .fetch_all(&pool)
        .await
        .expect("canonical archive query plan");
        assert!(plan.iter().any(|(_, _, _, detail)| {
            detail.contains("idx_archive_invocations_occurred_at")
                && detail.contains("occurred_at>? AND occurred_at<?")
        }));

        let control = LongTermProjectionWriteControl::unrestricted();
        let compatibility =
            inspect_long_term_archive_compatibility(&pool, &queries.parts, &control)
                .await
                .expect("archive compatibility probe");
        assert_eq!(
            compatibility,
            LongTermArchiveCompatibility {
                has_legacy_crossing: false,
                legacy_max_duration_ms: None,
                legacy_min_occurred_at: None,
                has_rfc3339: true,
                rfc3339_max_duration_ms: Some(2_000.0),
                rfc3339_min_occurred_at: Some("2026-07-25T02:00:01-14:00".to_string()),
            }
        );
        let rfc3339_compatibility = LongTermRfc3339Compatibility {
            max_duration_ms: compatibility.rfc3339_max_duration_ms,
        };
        let (rfc3339_lower, rfc3339_upper) =
            long_term_rfc3339_text_bounds(start, end, &rfc3339_compatibility);
        let rfc3339_plan = sqlx::query_as::<_, (i64, i64, i64, String)>(&format!(
            "EXPLAIN QUERY PLAN {}",
            queries.rfc3339
        ))
        .bind(&rfc3339_lower)
        .bind(&rfc3339_upper)
        .bind(start.timestamp())
        .bind(end.timestamp())
        .fetch_all(&pool)
        .await
        .expect("RFC3339 archive query plan");
        assert!(rfc3339_plan.iter().any(|(_, _, _, detail)| {
            detail.contains("idx_archive_invocations_occurred_at")
                && detail.contains("occurred_at>? AND occurred_at<?")
        }));
        let rows = load_long_term_archive_invocation_rows_for_range(
            &pool,
            &queries,
            compatibility,
            start,
            end,
        )
        .await
        .expect("archive range rows");
        let ids = rows.into_iter().map(|row| row.id).collect::<Vec<_>>();
        assert_eq!(ids, vec![1, 2, 5, 7, 8, 10]);
    }

    #[tokio::test]
    async fn archive_range_bounds_legacy_crossing_with_cached_max_duration() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, occurred_at TEXT NOT NULL, status TEXT, t_total_ms REAL)",
        )
        .execute(&pool)
        .await
        .expect("archive invocation schema");
        sqlx::query(
            "CREATE INDEX idx_archive_legacy_occurred_at ON codex_invocations (occurred_at)",
        )
        .execute(&pool)
        .await
        .expect("archive occurred_at index");
        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, t_total_ms) VALUES (1, '2026-01-01 00:00:00', 'success', 2000), (2, '2026-07-25 23:59:59', 'success', 2000), (3, '2026-07-26 12:00:00', 'success', 100), (4, '2026-02-01 00:00:00', 'success', 1e300)",
        )
        .execute(&pool)
        .await
        .expect("canonical archive rows");
        ensure_long_term_projection_schema(&pool)
            .await
            .expect("projection schema");
        let queries = long_term_archive_invocation_query_for_range(&pool)
            .await
            .expect("archive range queries");
        let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
        let start = Shanghai
            .from_local_datetime(&date.and_hms_opt(0, 0, 0).expect("day start"))
            .single()
            .expect("Shanghai day start");
        let end = Shanghai
            .from_local_datetime(
                &date
                    .succ_opt()
                    .expect("next date")
                    .and_hms_opt(0, 0, 0)
                    .expect("next day start"),
            )
            .single()
            .expect("Shanghai next day start");
        let control = LongTermProjectionWriteControl::unrestricted();
        let compatibility =
            inspect_long_term_archive_compatibility(&pool, &queries.parts, &control)
                .await
                .expect("archive compatibility probe");
        assert_eq!(
            compatibility,
            LongTermArchiveCompatibility {
                has_legacy_crossing: true,
                legacy_max_duration_ms: Some(1e300),
                legacy_min_occurred_at: Some("2026-01-01 00:00:00".to_string()),
                has_rfc3339: false,
                rfc3339_max_duration_ms: None,
                rfc3339_min_occurred_at: None,
            }
        );
        assert!(long_term_archive_legacy_crossing_start(&start, 1e300).is_none());
        let crossing_start = compatibility
            .legacy_min_occurred_at
            .as_deref()
            .expect("cached bounded legacy start");
        let start_text = start.format("%Y-%m-%d %H:%M:%S").to_string();
        let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(&format!(
            "EXPLAIN QUERY PLAN {}",
            queries.crossing_text
        ))
        .bind(crossing_start)
        .bind(&start_text)
        .fetch_all(&pool)
        .await
        .expect("bounded legacy crossing query plan");
        assert!(plan.iter().any(|(_, _, _, detail)| {
            detail.contains("idx_archive_legacy_occurred_at")
                && detail.contains("occurred_at>? AND occurred_at<?")
        }));

        persist_long_term_archive_compatibility(
            &pool,
            "legacy.sqlite.gz",
            "legacy-sha",
            "legacy-fingerprint",
            compatibility,
            &control,
        )
        .await
        .expect("persist legacy compatibility");
        let cached = load_long_term_archive_compatibility(
            &pool,
            "legacy.sqlite.gz",
            "legacy-sha",
            "legacy-fingerprint",
        )
        .await
        .expect("load legacy compatibility")
        .expect("cached legacy compatibility");
        let rows =
            load_long_term_archive_invocation_rows_for_range(&pool, &queries, cached, start, end)
                .await
                .expect("bounded legacy archive rows");
        assert_eq!(
            rows.into_iter().map(|row| row.id).collect::<Vec<_>>(),
            vec![2, 3, 4]
        );
    }

    #[tokio::test]
    async fn archive_range_uses_cached_canonical_capability_without_a_scan() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, occurred_at TEXT NOT NULL, status TEXT)",
        )
        .execute(&pool)
        .await
        .expect("archive invocation schema");
        sqlx::query(
            "CREATE INDEX idx_archive_standard_occurred_at ON codex_invocations (occurred_at)",
        )
        .execute(&pool)
        .await
        .expect("archive occurred_at index");
        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status) VALUES (1, '2026-07-26 12:00:00', 'success')",
        )
        .execute(&pool)
        .await
        .expect("standard archive row");
        let queries = long_term_archive_invocation_query_for_range(&pool)
            .await
            .expect("archive range queries");
        let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
        let start = Shanghai
            .from_local_datetime(&date.and_hms_opt(0, 0, 0).expect("day start"))
            .single()
            .expect("Shanghai day start");
        let end = Shanghai
            .from_local_datetime(
                &date
                    .succ_opt()
                    .expect("next date")
                    .and_hms_opt(0, 0, 0)
                    .expect("next day start"),
            )
            .single()
            .expect("Shanghai next day start");
        let start_text = start.format("%Y-%m-%d %H:%M:%S").to_string();
        let end_text = end.format("%Y-%m-%d %H:%M:%S").to_string();
        let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(&format!(
            "EXPLAIN QUERY PLAN {}",
            queries.canonical
        ))
        .bind(&start_text)
        .bind(&end_text)
        .fetch_all(&pool)
        .await
        .expect("canonical archive query plan");
        assert!(plan.iter().any(|(_, _, _, detail)| {
            detail.contains("idx_archive_standard_occurred_at")
                && detail.contains("occurred_at>? AND occurred_at<?")
        }));
        let rows = load_long_term_archive_invocation_rows_for_range(
            &pool,
            &queries,
            LongTermArchiveCompatibility {
                has_legacy_crossing: false,
                legacy_max_duration_ms: None,
                legacy_min_occurred_at: None,
                has_rfc3339: false,
                rfc3339_max_duration_ms: None,
                rfc3339_min_occurred_at: None,
            },
            start,
            end,
        )
        .await
        .expect("standard archive range rows");
        assert_eq!(rows.iter().map(|row| row.id).collect::<Vec<_>>(), vec![1]);
    }

    #[tokio::test]
    async fn archive_cleanup_safe_starts_preserve_nanosecond_cross_day_endpoints() {
        let occurred_at = "2026-07-25T15:59:59.9999999Z";
        let expected_safe_start = NaiveDate::from_ymd_opt(2026, 7, 27).expect("fixed date");
        let unique = format!(
            "{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let invocation_db_path = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-invocation-boundary-{unique}.sqlite"
        ));
        let invocation_archive_path = invocation_db_path.with_extension("sqlite.gz");
        fs::File::create(&invocation_db_path).expect("create invocation archive database");
        let invocation_options = format!("sqlite://{}", invocation_db_path.to_string_lossy())
            .parse::<SqliteConnectOptions>()
            .expect("parse invocation archive URL")
            .create_if_missing(true);
        let invocation_archive_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(invocation_options)
            .await
            .expect("open invocation archive database");
        sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, occurred_at TEXT NOT NULL, t_total_ms REAL)",
        )
        .execute(&invocation_archive_pool)
        .await
        .expect("create invocation archive schema");
        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, t_total_ms) VALUES (1, ?1, 0.001)",
        )
        .bind(occurred_at)
        .execute(&invocation_archive_pool)
        .await
        .expect("insert nanosecond invocation source");
        invocation_archive_pool.close().await;
        crate::maintenance::deflate_sqlite_file_to_gzip(
            &invocation_db_path,
            &invocation_archive_path,
        )
        .expect("compress invocation archive");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("full schema");
        let invocation_archive_sha256 =
            crate::maintenance::sha256_hex_file(&invocation_archive_path)
                .expect("hash invocation archive");
        sqlx::query(
            "INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, created_at) VALUES ('codex_invocations', '2026-07', ?1, ?2, 1, 'completed', datetime('now'))",
        )
        .bind(invocation_archive_path.to_string_lossy().to_string())
        .bind(&invocation_archive_sha256)
        .execute(&pool)
        .await
        .expect("record invocation archive manifest");
        assert_eq!(
            long_term_integrity_source_safe_start_for_archive_cleanup(
                &pool,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                invocation_archive_path.to_string_lossy().as_ref(),
                None,
            )
            .await
            .expect("read invocation archive boundary"),
            Some(expected_safe_start)
        );
        sqlx::query(
            "UPDATE archive_batches SET sha256 = 'stale-invocation-sha' WHERE file_path = ?1",
        )
        .bind(invocation_archive_path.to_string_lossy().to_string())
        .execute(&pool)
        .await
        .expect("stale invocation manifest");
        assert!(
            long_term_integrity_source_safe_start_for_archive_cleanup(
                &pool,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                invocation_archive_path.to_string_lossy().as_ref(),
                None,
            )
            .await
            .expect_err("mismatched invocation archive must not define a cleanup boundary")
            .to_string()
            .contains("does not match")
        );
        sqlx::query("UPDATE archive_batches SET sha256 = ?1 WHERE file_path = ?2")
            .bind(&invocation_archive_sha256)
            .bind(invocation_archive_path.to_string_lossy().to_string())
            .execute(&pool)
            .await
            .expect("restore invocation manifest");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, payload, raw_response, t_total_ms) VALUES (1, 'boundary-invoke', ?1, 'success', '{}', '{}', 0.001)",
        )
        .bind(occurred_at)
        .execute(&pool)
        .await
        .expect("insert live invocation source");

        let attempt_db_path = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-attempt-boundary-{unique}.sqlite"
        ));
        let attempt_archive_path = attempt_db_path.with_extension("sqlite.gz");
        fs::File::create(&attempt_db_path).expect("create attempt archive database");
        let attempt_options = format!("sqlite://{}", attempt_db_path.to_string_lossy())
            .parse::<SqliteConnectOptions>()
            .expect("parse attempt archive URL")
            .create_if_missing(true);
        let attempt_archive_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(attempt_options)
            .await
            .expect("open attempt archive database");
        sqlx::query(
            "CREATE TABLE pool_upstream_request_attempts (id INTEGER PRIMARY KEY, invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL, upstream_account_id INTEGER)",
        )
        .execute(&attempt_archive_pool)
        .await
        .expect("create attempt archive schema");
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, upstream_account_id) VALUES (1, 'boundary-invoke', ?1, 42)",
        )
        .bind(occurred_at)
        .execute(&attempt_archive_pool)
        .await
        .expect("insert attempt mapping");
        attempt_archive_pool.close().await;
        crate::maintenance::deflate_sqlite_file_to_gzip(&attempt_db_path, &attempt_archive_path)
            .expect("compress attempt archive");
        let attempt_archive_sha256 = crate::maintenance::sha256_hex_file(&attempt_archive_path)
            .expect("hash attempt archive");
        sqlx::query(
            "INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, created_at) VALUES ('pool_upstream_request_attempts', '2026-07', ?1, ?2, 1, 'completed', datetime('now'))",
        )
        .bind(attempt_archive_path.to_string_lossy().to_string())
        .bind(&attempt_archive_sha256)
        .execute(&pool)
        .await
        .expect("record attempt archive manifest");
        assert_eq!(
            long_term_integrity_source_safe_start_for_archive_cleanup(
                &pool,
                "pool_upstream_request_attempts",
                attempt_archive_path.to_string_lossy().as_ref(),
                None,
            )
            .await
            .expect("read attempt archive boundary"),
            Some(expected_safe_start)
        );

        for path in [
            invocation_db_path,
            invocation_archive_path,
            attempt_db_path,
            attempt_archive_path,
        ] {
            let _ = fs::remove_file(path);
        }
    }

    #[tokio::test]
    async fn invocation_correction_wakes_a_deferred_projection_repair() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, source TEXT, status TEXT, occurred_at TEXT, model TEXT, payload TEXT, input_tokens INTEGER, output_tokens INTEGER, cache_input_tokens INTEGER, reasoning_tokens INTEGER, total_tokens INTEGER, cost REAL, t_total_ms REAL, t_req_read_ms REAL, t_req_parse_ms REAL, t_upstream_connect_ms REAL, t_upstream_ttfb_ms REAL, t_upstream_stream_ms REAL, error_message TEXT, failure_kind TEXT)",
        )
        .execute(&pool)
        .await
        .expect("invocation schema");
        ensure_long_term_projection_schema(&pool)
            .await
            .expect("projection schema");
        ensure_long_term_projection_correction_trigger(&pool)
            .await
            .expect("correction trigger");
        sqlx::query("INSERT INTO codex_invocations (id, occurred_at, status, model) VALUES (1, '2026-07-26 10:00:00', 'success', 'before')")
            .execute(&pool)
            .await
            .expect("invocation");
        queue_long_term_projection_repairs(
            &pool,
            &["2026-07-26".to_string()],
            "source_unavailable",
        )
        .await
        .expect("dirty bucket");
        defer_long_term_projection_repair(&pool, "2026-07-26")
            .await
            .expect("defer repair");

        sqlx::query("UPDATE codex_invocations SET model = 'after' WHERE id = 1")
            .execute(&pool)
            .await
            .expect("correct invocation");

        let next_attempt_at = sqlx::query_scalar::<_, Option<String>>(
            "SELECT next_attempt_at FROM long_term_projection_dirty_buckets WHERE bucket_date = '2026-07-26'",
        )
        .fetch_one(&pool)
        .await
        .expect("repair marker");
        assert!(next_attempt_at.is_none());
    }

    #[tokio::test]
    async fn terminal_finalize_does_not_enqueue_a_long_term_date_rebuild() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, source TEXT, status TEXT, occurred_at TEXT, model TEXT, payload TEXT, input_tokens INTEGER, output_tokens INTEGER, cache_input_tokens INTEGER, reasoning_tokens INTEGER, total_tokens INTEGER, cost REAL, t_total_ms REAL, t_req_read_ms REAL, t_req_parse_ms REAL, t_upstream_connect_ms REAL, t_upstream_ttfb_ms REAL, t_upstream_stream_ms REAL, error_message TEXT, failure_kind TEXT)",
        )
        .execute(&pool)
        .await
        .expect("invocation schema");
        ensure_long_term_projection_schema(&pool)
            .await
            .expect("projection schema");
        ensure_long_term_projection_correction_trigger(&pool)
            .await
            .expect("correction trigger");
        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, model) VALUES (1, '2026-07-26 10:00:00', 'running', 'gpt-5')",
        )
        .execute(&pool)
        .await
        .expect("running invocation");

        sqlx::query("UPDATE codex_invocations SET status = 'success' WHERE id = 1")
            .execute(&pool)
            .await
            .expect("terminal finalize");
        let dirty_count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_dirty_buckets")
                .fetch_one(&pool)
                .await
                .expect("dirty count");
        assert_eq!(dirty_count, 0);

        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, failure_kind, model) VALUES (2, '2026-07-26 11:00:00', 'interrupted', 'proxy_interrupted', 'gpt-5')",
        )
        .execute(&pool)
        .await
        .expect("recoverable interrupted invocation");
        sqlx::query(
            "UPDATE codex_invocations SET status = 'success', failure_kind = NULL WHERE id = 2",
        )
        .execute(&pool)
        .await
        .expect("recovered terminal finalize");
        let dirty_count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_dirty_buckets")
                .fetch_one(&pool)
                .await
                .expect("dirty count");
        assert_eq!(dirty_count, 0);

        sqlx::query("UPDATE codex_invocations SET model = 'gpt-5.1' WHERE id = 1")
            .execute(&pool)
            .await
            .expect("historical correction");
        let dirty_count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_dirty_buckets")
                .fetch_one(&pool)
                .await
                .expect("dirty count");
        assert_eq!(dirty_count, 1);
    }

    #[tokio::test]
    async fn out_of_order_terminal_finalize_queues_an_exact_date_rebuild() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, source TEXT, status TEXT, occurred_at TEXT, model TEXT, payload TEXT, input_tokens INTEGER, output_tokens INTEGER, cache_input_tokens INTEGER, reasoning_tokens INTEGER, total_tokens INTEGER, cost REAL, t_total_ms REAL, t_req_read_ms REAL, t_req_parse_ms REAL, t_upstream_connect_ms REAL, t_upstream_ttfb_ms REAL, t_upstream_stream_ms REAL, error_message TEXT, failure_kind TEXT)",
        )
        .execute(&pool)
        .await
        .expect("invocation schema");
        ensure_long_term_projection_schema(&pool)
            .await
            .expect("projection schema");
        ensure_long_term_projection_correction_trigger(&pool)
            .await
            .expect("correction trigger");
        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, model) VALUES (1, '2026-07-26 10:00:00', 'running', 'gpt-5'), (2, '2026-07-26 11:00:00', 'success', 'gpt-5')",
        )
        .execute(&pool)
        .await
        .expect("out-of-order source rows");
        sqlx::query(
            "INSERT INTO long_term_projection_state (consumer, cursor_row_id) VALUES (?1, 2)",
        )
        .bind(LONG_TERM_PROJECTION_CONSUMER)
        .execute(&pool)
        .await
        .expect("advanced projection cursor");

        sqlx::query("UPDATE codex_invocations SET status = 'success' WHERE id = 1")
            .execute(&pool)
            .await
            .expect("late terminal finalize");
        let dirty_count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_dirty_buckets")
                .fetch_one(&pool)
                .await
                .expect("dirty count");
        assert_eq!(dirty_count, 1);
    }

    #[tokio::test]
    async fn cursor_repair_defer_applies_to_every_affected_date() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_projection_schema(&pool)
            .await
            .expect("projection schema");
        let dates = vec!["2026-07-26".to_string(), "2026-07-27".to_string()];
        queue_long_term_projection_repairs(&pool, &dates, "interval_baseline")
            .await
            .expect("dirty buckets");

        defer_long_term_projection_repairs(&pool, &dates)
            .await
            .expect("defer repairs");
        ensure_long_term_projection_repairs(&pool, &dates, "interval_baseline")
            .await
            .expect("ensure existing repairs");
        assert!(
            long_term_projection_repairs_are_deferred(&pool, &dates)
                .await
                .expect("load repair deadline")
        );

        let deferred = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_projection_dirty_buckets WHERE next_attempt_at IS NOT NULL AND datetime(next_attempt_at) > datetime('now')",
        )
        .fetch_one(&pool)
        .await
        .expect("deferred markers");
        assert_eq!(deferred, 2);
    }

    #[tokio::test]
    async fn archive_trigger_invalidates_month_when_coverage_is_unknown() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        sqlx::query(
            "CREATE TABLE archive_batches (id INTEGER PRIMARY KEY, dataset TEXT NOT NULL, month_key TEXT NOT NULL, file_path TEXT NOT NULL, sha256 TEXT NOT NULL, status TEXT NOT NULL, coverage_start_at TEXT, coverage_end_at TEXT, historical_rollups_materialized_at TEXT)",
        )
        .execute(&pool)
        .await
        .expect("archive schema");
        ensure_long_term_projection_schema(&pool)
            .await
            .expect("projection schema");
        ensure_long_term_projection_archive_trigger(&pool)
            .await
            .expect("archive trigger");

        sqlx::query(
            "INSERT INTO archive_batches (id, dataset, month_key, file_path, sha256, status) VALUES (1, 'codex_invocations', '2026-07', 'legacy.db', 'sha', 'completed')",
        )
        .execute(&pool)
        .await
        .expect("unknown coverage archive");

        let dates = sqlx::query_scalar::<_, String>(
            "SELECT bucket_date FROM long_term_projection_dirty_buckets ORDER BY bucket_date",
        )
        .fetch_all(&pool)
        .await
        .expect("dirty dates");
        assert_eq!(dates.len(), 31);
        assert_eq!(dates.first().map(String::as_str), Some("2026-07-01"));
        assert_eq!(dates.last().map(String::as_str), Some("2026-07-31"));

        sqlx::query("DELETE FROM long_term_projection_dirty_buckets")
            .execute(&pool)
            .await
            .expect("clear month fallback markers");
        sqlx::query(
            "INSERT INTO archive_batches (id, dataset, month_key, file_path, sha256, status, coverage_start_at, coverage_end_at) VALUES (2, 'codex_invocations', '2026-07', 'fractional.db', 'fractional-sha', 'completed', '2026-07-25T15:59:59.9999Z', '2026-07-25T16:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("fractional RFC3339 archive coverage");
        let dates = sqlx::query_scalar::<_, String>(
            "SELECT bucket_date FROM long_term_projection_dirty_buckets ORDER BY bucket_date",
        )
        .fetch_all(&pool)
        .await
        .expect("fractional archive dirty dates");
        assert_eq!(dates, vec!["2026-07-25", "2026-07-26"]);
    }

    #[tokio::test]
    async fn accepted_projection_baseline_recovers_the_public_stats_state() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
            .bind(LONG_TERM_STATUS_RUNNING)
            .bind(LONG_TERM_STATE_ID)
            .execute(&pool)
            .await
            .expect("interrupted state");

        commit_long_term_projection_date_rebuilds(&pool, &[], Some(7), &[], true)
            .await
            .expect("accepted baseline");

        let status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("recovered state");
        assert_eq!(status, LONG_TERM_STATUS_READY);

        sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
            .bind(LONG_TERM_STATUS_ERROR)
            .bind(LONG_TERM_STATE_ID)
            .execute(&pool)
            .await
            .expect("integrity error state");
        commit_long_term_projection_date_rebuilds(&pool, &[], Some(8), &[], false)
            .await
            .expect("baseline while error is retained");
        let status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("retained error state");
        assert_eq!(status, LONG_TERM_STATUS_ERROR);
    }

    #[tokio::test]
    async fn nonempty_projection_repair_recovers_empty_state_and_advances_start_date() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        create_long_term_test_invocations(&pool).await;
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, statistics_start_date = '2026-07-27' WHERE id = ?2",
        )
        .bind(LONG_TERM_STATUS_EMPTY)
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("empty state");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (1, 'first-repaired', '2026-07-26 10:00:00', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("first repaired invocation");

        let control = LongTermProjectionWriteControl::unrestricted();
        let rebuild = build_long_term_projection_date_rebuild(&pool, "2026-07-26", &control)
            .await
            .expect("date rebuild");
        commit_long_term_projection_date_rebuilds(&pool, &[rebuild], Some(1), &[], false)
            .await
            .expect("repair commit");

        let (status, start_date) = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT status, statistics_start_date FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("repaired state");
        assert_eq!(status, LONG_TERM_STATUS_READY);
        assert_eq!(start_date.as_deref(), Some("2026-07-26"));
    }

    #[tokio::test]
    async fn incremental_projection_preserves_an_existing_error_state() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        ensure_long_term_stats_schema(&pool)
            .await
            .expect("long-term schema");
        sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, last_error = 'source unavailable' WHERE id = ?2",
        )
        .bind(LONG_TERM_STATUS_ERROR)
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("error state");
        queue_long_term_projection_repairs(
            &pool,
            &["2026-07-25".to_string()],
            "source_unavailable",
        )
        .await
        .expect("dirty bucket");
        let runtime = Arc::new(Mutex::new(LongTermProjectionRuntime::default()));

        apply_long_term_projection_incremental_with_runtime(
            &pool,
            &runtime,
            &HashMap::new(),
            &HashMap::new(),
            &[],
            7,
            1,
        )
        .await
        .expect("incremental flush");

        let (status, last_error) = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT status, last_error FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("preserved state");
        assert_eq!(status, LONG_TERM_STATUS_ERROR);
        assert_eq!(last_error.as_deref(), Some("source unavailable"));
    }

    #[tokio::test]
    async fn account_kind_change_invalidates_affected_projection_dates() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        create_long_term_test_invocations(&pool).await;
        sqlx::query("CREATE TABLE pool_upstream_accounts (id INTEGER PRIMARY KEY, kind TEXT)")
            .execute(&pool)
            .await
            .expect("account schema");
        sqlx::query("CREATE TABLE pool_upstream_request_attempts (id INTEGER PRIMARY KEY, invoke_id TEXT, occurred_at TEXT, upstream_account_id INTEGER)")
            .execute(&pool)
            .await
            .expect("attempt schema");
        sqlx::query("CREATE TABLE archive_batches (id INTEGER PRIMARY KEY, dataset TEXT NOT NULL, month_key TEXT NOT NULL, status TEXT NOT NULL, coverage_start_at TEXT, coverage_end_at TEXT)")
            .execute(&pool)
            .await
            .expect("archive schema");
        ensure_long_term_projection_schema(&pool)
            .await
            .expect("projection schema");
        ensure_long_term_projection_account_trigger(&pool)
            .await
            .expect("account trigger");
        sqlx::query("INSERT INTO pool_upstream_accounts (id, kind) VALUES (42, 'oauth_codex')")
            .execute(&pool)
            .await
            .expect("account");
        sqlx::query("INSERT INTO archive_batches (id, dataset, month_key, status, coverage_start_at, coverage_end_at) VALUES (1, 'codex_invocations', '2026-06', 'completed', '2026-06-01 00:00:00', '2026-06-02 23:59:59')")
            .execute(&pool)
            .await
            .expect("archived coverage");
        sqlx::query("INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens, payload) VALUES (1, 'classified-before-day-start', '2026-07-25T15:59:59.9999Z', 'success', 100, '{\"upstreamAccountId\":42}')")
            .execute(&pool)
            .await
            .expect("classified sub-millisecond pre-start invocation");
        sqlx::query("INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens, payload) VALUES (2, 'classified-before-next-day-start', '2026-07-26T15:59:59.9999Z', 'success', 100, '{\"upstreamAccountId\":42}')")
            .execute(&pool)
            .await
            .expect("classified sub-millisecond pre-end invocation");

        sqlx::query("UPDATE pool_upstream_accounts SET kind = 'api_key' WHERE id = 42")
            .execute(&pool)
            .await
            .expect("classification update");

        let active_dates = sqlx::query_scalar::<_, String>(
            "SELECT bucket_date FROM long_term_projection_dirty_buckets WHERE bucket_date LIKE '2026-07-%' ORDER BY bucket_date",
        )
        .fetch_all(&pool)
        .await
        .expect("affected dates");
        assert_eq!(active_dates, vec!["2026-07-25", "2026-07-26"]);
        let archived_dates = sqlx::query_scalar::<_, String>(
            "SELECT bucket_date FROM long_term_projection_dirty_buckets WHERE bucket_date LIKE '2026-06-%' ORDER BY bucket_date",
        )
        .fetch_all(&pool)
        .await
        .expect("archived dates");
        assert_eq!(archived_dates, vec!["2026-06-01", "2026-06-02"]);
    }
}
