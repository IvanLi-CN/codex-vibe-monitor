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
