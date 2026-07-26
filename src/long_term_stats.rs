use super::*;

const LONG_TERM_TIMEZONE: &str = "Asia/Shanghai";
const LONG_TERM_STATE_ID: i64 = 1;
const LONG_TERM_STATUS_DISABLED: &str = "disabled";
const LONG_TERM_STATUS_PREPARING: &str = "preparing";
const LONG_TERM_STATUS_RUNNING: &str = "running";
const LONG_TERM_STATUS_READY: &str = "ready";
const LONG_TERM_STATUS_EMPTY: &str = "empty";
const LONG_TERM_STATUS_ERROR: &str = "error";
const LONG_TERM_OTHER_KEY: &str = "other";
const LONG_TERM_OTHER_NAME: &str = "其他";
const LONG_TERM_HOUR_MS: i64 = 60 * 60 * 1000;
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
    processed_rows: i64,
    total_rows: i64,
    last_error: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
struct LongTermInvocationRow {
    id: i64,
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
    "long_term_usage_hourly, long_term_usage_daily and long_term_stats_state"
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
    Ok(())
}

pub(crate) fn spawn_long_term_stats_backfill(
    pool: Pool<Sqlite>,
    retention_days: u64,
    shutdown: CancellationToken,
) {
    tokio::spawn(async move {
        let _ = sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, updated_at = datetime('now') WHERE id = ?2 AND status <> ?3",
        )
        .bind(LONG_TERM_STATUS_PREPARING)
        .bind(LONG_TERM_STATE_ID)
        .bind(LONG_TERM_STATUS_READY)
        .execute(&pool)
        .await;
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
    let refresh_started_at = format_utc_iso(Utc::now());
    let state_snapshot = sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT status, statistics_start_date FROM long_term_stats_state WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .fetch_optional(pool)
    .await?;
    let was_ready = state_snapshot
        .as_ref()
        .is_some_and(|(status, _)| status.as_deref() == Some(LONG_TERM_STATUS_READY));
    if !was_ready {
        sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, last_error = NULL, updated_at = datetime('now') WHERE id = ?2",
        )
        .bind(LONG_TERM_STATUS_RUNNING)
        .bind(LONG_TERM_STATE_ID)
        .execute(pool)
        .await?;
    } else {
        sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, last_error = NULL, updated_at = datetime('now') WHERE id = ?2",
        )
        .bind(LONG_TERM_STATUS_READY)
        .bind(LONG_TERM_STATE_ID)
        .execute(pool)
        .await?;
    }

    let result =
        refresh_long_term_stats_inner(pool, retention_days, was_ready, &refresh_started_at).await;
    if let Err(err) = &result {
        let _ = sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, last_error = ?2, updated_at = datetime('now') WHERE id = ?3 AND NOT (status = ?4 AND datetime(updated_at) > datetime(?5))",
        )
        .bind(if was_ready {
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
        "SELECT status, statistics_start_date, processed_rows, total_rows, last_error FROM long_term_stats_state WHERE id = ?1",
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
        "COALESCE(CASE WHEN json_valid(inv.payload) THEN CAST(json_extract(inv.payload, '$.upstreamAccountId') AS INTEGER) END, (SELECT attempt.upstream_account_id FROM pool_upstream_request_attempts attempt WHERE attempt.invoke_id = inv.invoke_id AND attempt.upstream_account_id IS NOT NULL ORDER BY attempt.attempt_index DESC, attempt.id DESC LIMIT 1))"
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
    let mut hourly: HashMap<(i64, String, String), LongTermBucket> = HashMap::new();
    let mut daily: HashMap<(String, String, String), LongTermBucket> = HashMap::new();
    let mut statistics_start_date = previous_state
        .as_ref()
        .and_then(|state| state.statistics_start_date.clone());
    let account_identities = load_long_term_account_identities(pool).await?;
    let mut rows = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut processed_rows_count = 0_i64;
    if ready_state {
        let live_sql = format!(
            r#"
        SELECT
            inv.id,
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
            if seen_ids.insert(row.id) {
                rows.push(row);
            }
        }
    } else {
        let live_sql = format!(
            r#"
        SELECT
            inv.id,
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
        while let Some(mut row) = live_rows.try_next().await? {
            if seen_ids.insert(row.id) {
                hydrate_long_term_account_identity(&mut row, &account_identities);
                accumulate_long_term_invocation(
                    &row,
                    &mut hourly,
                    &mut daily,
                    &mut statistics_start_date,
                );
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
    let all_archive_paths = archive_paths.clone();
    let mut archive_markers = Vec::new();
    let mut archive_read_failed = false;
    let mut unavailable_after_date: Option<NaiveDate> = None;
    let mut affected_archive_dates = HashSet::new();
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
                warn!(error = %error, file_path = archive_path.file_path(), "long-term stats archive read failed");
                None
            }
        }) else {
            if let Some(date) = archive_path
                .coverage_end_at()
                .and_then(long_term_archive_end_date)
            {
                unavailable_after_date =
                    Some(unavailable_after_date.map_or(date, |current| current.max(date)));
            }
            continue;
        };
        let archive_rows = sqlx::query_as::<_, LongTermInvocationRow>(
            r#"
            SELECT
                id,
                occurred_at,
                status,
                model,
                CASE WHEN json_valid(payload) THEN NULLIF(TRIM(CAST(json_extract(payload, '$.requestModel') AS TEXT)), '') END AS request_model,
                CASE WHEN json_valid(payload) THEN NULLIF(TRIM(CAST(json_extract(payload, '$.responseModel') AS TEXT)), '') END AS response_model,
                CASE WHEN json_valid(payload) THEN NULLIF(TRIM(CAST(json_extract(payload, '$.reasoningEffort') AS TEXT)), '') END AS reasoning_effort,
                CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END AS upstream_account_id,
                NULL AS upstream_account_kind,
                NULL AS upstream_account_name,
                total_tokens,
                output_tokens,
                cost,
                t_total_ms,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                t_upstream_stream_ms,
                error_message
            FROM codex_invocations
            WHERE LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')
            ORDER BY occurred_at ASC, id ASC
            "#,
        )
        .fetch_all(&archive_pool)
        .await;
        archive_pool.close().await;
        drop(cleanup);
        match archive_rows {
            Ok(archive_rows) => {
                for row in archive_rows {
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
                    if seen_ids.insert(row.id) {
                        processed_rows_count += 1;
                        let mut row = row;
                        hydrate_long_term_account_identity(&mut row, &account_identities);
                        if ready_state {
                            rows.push(row);
                        } else {
                            accumulate_long_term_invocation(
                                &row,
                                &mut hourly,
                                &mut daily,
                                &mut statistics_start_date,
                            );
                        }
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
                warn!(error = %error, file_path = archive_path.file_path(), "long-term stats archive query failed");
                if let Some(date) = archive_path
                    .coverage_end_at()
                    .and_then(long_term_archive_end_date)
                {
                    unavailable_after_date =
                        Some(unavailable_after_date.map_or(date, |current| current.max(date)));
                }
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

    // A day may be split across live rows and archive parts. Rebuild every date touched by the
    // current live tail from all overlapping source parts before replacing durable buckets.
    let mut recomputed_dates = affected_archive_dates.clone();
    for (date, _, _) in daily.keys() {
        if let Ok(date) = NaiveDate::parse_from_str(date, "%Y-%m-%d") {
            recomputed_dates.insert(date);
        }
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
                    inv.id, inv.occurred_at, inv.status, inv.model,
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
            let archive_rows = sqlx::query_as::<_, LongTermInvocationRow>(
                r#"
                SELECT
                    id, occurred_at, status, model,
                    CASE WHEN json_valid(payload) THEN NULLIF(TRIM(CAST(json_extract(payload, '$.requestModel') AS TEXT)), '') END AS request_model,
                    CASE WHEN json_valid(payload) THEN NULLIF(TRIM(CAST(json_extract(payload, '$.responseModel') AS TEXT)), '') END AS response_model,
                    CASE WHEN json_valid(payload) THEN NULLIF(TRIM(CAST(json_extract(payload, '$.reasoningEffort') AS TEXT)), '') END AS reasoning_effort,
                    CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END AS upstream_account_id,
                    NULL AS upstream_account_kind, NULL AS upstream_account_name,
                    total_tokens, output_tokens, cost, t_total_ms, t_req_read_ms,
                    t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms,
                    t_upstream_stream_ms, error_message
                FROM codex_invocations
                WHERE LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')
                ORDER BY occurred_at ASC, id ASC
                "#,
            )
            .fetch_all(&archive_pool)
            .await?;
            archive_pool.close().await;
            drop(cleanup);
            for row in archive_rows {
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
    let mut statistics_start_date =
        statistics_start_date.and_then(|date| NaiveDate::parse_from_str(&date, "%Y-%m-%d").ok());
    if let Some(unavailable_after_date) = unavailable_after_date {
        let safe_start = unavailable_after_date
            .succ_opt()
            .unwrap_or(unavailable_after_date);
        statistics_start_date = Some(
            statistics_start_date
                .map(|date| date.max(safe_start))
                .unwrap_or(safe_start),
        );
    }
    // A damaged archive only makes the older portion unavailable. Keep the materialized
    // suffix readable from the safe start date and leave the archive unmarked so the next
    // refresh can retry it. Without coverage metadata there is no safe truncation point.
    let status = if archive_read_failed && unavailable_after_date.is_none() {
        LONG_TERM_STATUS_ERROR
    } else if rows.is_empty() && daily.is_empty() && !has_persisted_daily_rows {
        if archive_read_failed {
            LONG_TERM_STATUS_READY
        } else {
            LONG_TERM_STATUS_EMPTY
        }
    } else {
        LONG_TERM_STATUS_READY
    };
    sqlx::query(
        "UPDATE long_term_stats_state SET status = ?1, statistics_start_date = ?2, processed_rows = ?3, total_rows = ?3, last_error = ?4, updated_at = datetime('now') WHERE id = ?5 AND NOT (status = ?6 AND datetime(updated_at) > datetime(?7))",
    )
    .bind(status)
    .bind(statistics_start_date.map(|date| date.to_string()))
    .bind(if ready_state { rows.len() as i64 } else { processed_rows_count })
    .bind(archive_read_failed.then_some("one or more invocation archives could not be materialized"))
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
        let latest = matching.last();
        series.push(LongTermSeries {
            series_key: key,
            display_name: latest
                .map(|row| row.display_name.clone())
                .unwrap_or_default(),
            reasoning_effort: latest.and_then(|row| {
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
        "SELECT status, statistics_start_date, processed_rows, total_rows, last_error FROM long_term_stats_state WHERE id = ?1",
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

    #[test]
    fn metrics_keep_call_count_separate_from_success_only_timing_samples() {
        let success = LongTermInvocationRow {
            id: 1,
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
    async fn refresh_materializes_daily_rows_without_account_or_archive_tables() {
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
        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, model, payload, total_tokens, output_tokens, cost, t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms) VALUES (1, datetime('now'), 'success', 'gpt-5', '{\"reasoningEffort\":\"high\"}', 12, 4, 0.2, 100, 10, 5, 5, 20, 80)",
        )
        .execute(&pool)
        .await
        .expect("invocation row");
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
            .expect("refresh after source retention");
        let retained_daily_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_usage_daily WHERE dimension = 'overall'",
        )
        .fetch_one(&pool)
        .await
        .expect("retained daily count");
        assert_eq!(retained_daily_rows, 1);
        let retained_hourly_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_usage_hourly WHERE dimension = 'overall'",
        )
        .fetch_one(&pool)
        .await
        .expect("retained hourly count");
        assert_eq!(retained_hourly_rows, 1);
        let status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM long_term_stats_state WHERE id = 1",
        )
        .fetch_one(&pool)
        .await
        .expect("state");
        assert_eq!(status, LONG_TERM_STATUS_READY);
    }
}
