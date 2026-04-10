#[derive(Debug, Default)]
struct RetentionRunSummary {
    dry_run: bool,
    raw_files_compression_candidates: usize,
    raw_files_compressed: usize,
    raw_bytes_before: u64,
    raw_bytes_after: u64,
    raw_bytes_after_estimated: u64,
    invocation_details_pruned: usize,
    invocation_rows_archived: usize,
    forward_proxy_attempt_rows_archived: usize,
    pool_upstream_request_attempt_rows_archived: usize,
    stats_source_snapshot_rows_archived: usize,
    quota_snapshot_rows_archived: usize,
    archive_batches_touched: usize,
    archive_batches_deleted: usize,
    raw_files_removed: usize,
    orphan_raw_files_removed: usize,
}

impl RetentionRunSummary {
    fn touched_anything(&self) -> bool {
        self.raw_files_compression_candidates > 0
            || self.raw_files_compressed > 0
            || self.invocation_details_pruned > 0
            || self.invocation_rows_archived > 0
            || self.forward_proxy_attempt_rows_archived > 0
            || self.pool_upstream_request_attempt_rows_archived > 0
            || self.stats_source_snapshot_rows_archived > 0
            || self.quota_snapshot_rows_archived > 0
            || self.archive_batches_deleted > 0
            || self.raw_files_removed > 0
            || self.orphan_raw_files_removed > 0
    }
}

#[derive(Debug, Clone, Copy)]
struct ArchiveTableSpec {
    dataset: &'static str,
    columns: &'static str,
    create_sql: &'static str,
}

#[derive(Debug)]
struct ArchiveBatchOutcome {
    dataset: &'static str,
    month_key: String,
    day_key: Option<String>,
    part_key: Option<String>,
    file_path: String,
    sha256: String,
    row_count: i64,
    upstream_last_activity: Vec<(i64, String)>,
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
    archive_expires_at: Option<String>,
    layout: &'static str,
    codec: &'static str,
    writer_version: &'static str,
    cleanup_state: &'static str,
    superseded_by: Option<i64>,
}

#[derive(Debug, Default)]
struct InvocationRollupDelta {
    total_count: i64,
    success_count: i64,
    failure_count: i64,
    total_tokens: i64,
    total_cost: f64,
}

#[derive(Debug, FromRow)]
struct InvocationDetailPruneCandidate {
    id: i64,
    occurred_at: String,
    request_raw_path: Option<String>,
    response_raw_path: Option<String>,
}

#[derive(Debug, FromRow, Clone)]
struct InvocationArchiveCandidate {
    id: i64,
    occurred_at: String,
    source: String,
    status: Option<String>,
    total_tokens: Option<i64>,
    cost: Option<f64>,
    request_raw_path: Option<String>,
    response_raw_path: Option<String>,
}

#[derive(Debug, FromRow, Clone)]
struct InvocationRawCompressionFieldCandidate {
    id: i64,
    occurred_at: String,
    raw_path: String,
}

#[derive(Debug, FromRow)]
struct ArchiveBatchFileRow {
    id: i64,
    file_path: String,
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
}

#[derive(Debug, FromRow)]
struct InvocationBucketPresenceRow {
    occurred_at: String,
    source: String,
}

#[derive(Debug, FromRow)]
struct ArchiveManifestBatchRow {
    id: i64,
    file_path: String,
}

#[derive(Debug, FromRow)]
struct ArchiveStorageManifestRow {
    id: i64,
    dataset: String,
    layout: String,
    file_path: String,
}

#[derive(Debug, Default)]
struct ArchiveTempCleanupSummary {
    stale_temp_files_removed: usize,
    stale_temp_bytes_removed: u64,
}

#[derive(Debug, Default)]
struct ArchiveStorageVerificationSummary {
    manifest_rows: usize,
    missing_files: usize,
    orphan_files: usize,
    stale_temp_files: usize,
    stale_temp_bytes: u64,
}

#[derive(Debug, Default)]
struct ArchiveBatchPruneSummary {
    expired_archive_batches_deleted: usize,
    legacy_archive_batches_deleted: usize,
}

#[derive(Debug, FromRow)]
struct RawCompressionBacklogAggRow {
    uncompressed_count: i64,
    uncompressed_bytes: Option<i64>,
    oldest_occurred_at: Option<String>,
}

#[derive(Debug, FromRow)]
struct ArchivedAccountLastActivityRow {
    account_id: i64,
    last_activity_at: String,
}

fn dedupe_archive_upstream_last_activity(
    values: impl IntoIterator<Item = (i64, String)>,
) -> Vec<(i64, String)> {
    let mut deduped = BTreeMap::<i64, String>::new();
    for (account_id, last_activity_at) in values {
        deduped
            .entry(account_id)
            .and_modify(|current| {
                if *current < last_activity_at {
                    *current = last_activity_at.clone();
                }
            })
            .or_insert(last_activity_at);
    }
    deduped.into_iter().collect()
}

#[derive(Debug, Default)]
struct ArchiveBackfillSummary {
    scanned_batches: u64,
    updated_accounts: u64,
    hit_budget: bool,
    waiting_for_manifest_backfill: bool,
}

#[allow(dead_code)]
#[derive(Debug, Default)]
pub(crate) struct HistoricalRollupMaterializationSummary {
    scanned_archive_batches: usize,
    materialized_archive_batches: usize,
    materialized_bucket_count: usize,
    materialized_invocation_batches: usize,
    materialized_forward_proxy_batches: usize,
    last_materialized_bucket_start_epoch: Option<i64>,
}

#[allow(dead_code)]
#[derive(Debug, Default)]
pub(crate) struct LegacyArchivePruneSummary {
    scanned_archive_batches: usize,
    deleted_archive_batches: usize,
    skipped_unmaterialized_batches: usize,
    skipped_retained_batches: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum HistoricalRollupBackfillAlertLevel {
    None,
    Warn,
    Critical,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HistoricalRollupBackfillSnapshot {
    pub(crate) pending_buckets: u64,
    pub(crate) legacy_archive_pending: u64,
    pub(crate) last_materialized_hour: Option<String>,
    pub(crate) alert_level: HistoricalRollupBackfillAlertLevel,
}

const HOURLY_ROLLUP_DATASET_INVOCATIONS: &str = "codex_invocations";
const HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS: &str = "forward_proxy_attempts";
const HOURLY_ROLLUP_TARGET_INVOCATIONS: &str = "invocation_rollup_hourly";
const HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES: &str = "invocation_failure_rollup_hourly";
const HOURLY_ROLLUP_TARGET_PROXY_PERF: &str = "proxy_perf_stage_hourly";
const HOURLY_ROLLUP_TARGET_PROMPT_CACHE: &str = "prompt_cache_rollup_hourly";
const HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS: &str =
    "prompt_cache_upstream_account_hourly";
const HOURLY_ROLLUP_TARGET_STICKY_KEYS: &str = "upstream_sticky_key_hourly";
const HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS: &str = "forward_proxy_attempt_hourly";
const HISTORICAL_ROLLUP_ARCHIVE_DATASETS: [&str; 2] = [
    HOURLY_ROLLUP_DATASET_INVOCATIONS,
    HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
];
const INVOCATION_HOURLY_ROLLUP_TARGETS: [&str; 6] = [
    HOURLY_ROLLUP_TARGET_INVOCATIONS,
    HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
    HOURLY_ROLLUP_TARGET_PROXY_PERF,
    HOURLY_ROLLUP_TARGET_PROMPT_CACHE,
    HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS,
    HOURLY_ROLLUP_TARGET_STICKY_KEYS,
];
const PERF_STAGE_TOTAL: &str = "total";
const PERF_STAGE_REQUEST_READ: &str = "requestRead";
const PERF_STAGE_REQUEST_PARSE: &str = "requestParse";
const PERF_STAGE_UPSTREAM_CONNECT: &str = "upstreamConnect";
const PERF_STAGE_UPSTREAM_FIRST_BYTE: &str = "upstreamFirstByte";
const PERF_STAGE_UPSTREAM_STREAM: &str = "upstreamStream";
const PERF_STAGE_RESPONSE_PARSE: &str = "responseParse";
const PERF_STAGE_PERSISTENCE: &str = "persistence";
const HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE: &str = "";

#[derive(Debug, Clone, FromRow)]
struct InvocationHourlySourceRecord {
    id: i64,
    occurred_at: String,
    source: String,
    status: Option<String>,
    detail_level: String,
    total_tokens: Option<i64>,
    cost: Option<f64>,
    error_message: Option<String>,
    failure_kind: Option<String>,
    failure_class: Option<String>,
    is_actionable: Option<i64>,
    payload: Option<String>,
    t_total_ms: Option<f64>,
    t_req_read_ms: Option<f64>,
    t_req_parse_ms: Option<f64>,
    t_upstream_connect_ms: Option<f64>,
    t_upstream_ttfb_ms: Option<f64>,
    t_upstream_stream_ms: Option<f64>,
    t_resp_parse_ms: Option<f64>,
    t_persist_ms: Option<f64>,
}

#[derive(Debug, Clone, FromRow)]
struct ForwardProxyAttemptHourlySourceRecord {
    id: i64,
    proxy_key: String,
    occurred_at: String,
    is_success: i64,
    latency_ms: Option<f64>,
}

#[derive(Debug)]
struct TempSqliteCleanup(PathBuf);

impl Drop for TempSqliteCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn sqlite_url_for_path(path: &Path) -> String {
    format!("sqlite://{}", path.to_string_lossy())
}

#[derive(Debug, Default)]
struct RawCompressionPassSummary {
    files_considered: usize,
    files_compressed: usize,
    bytes_before: u64,
    bytes_after: u64,
    estimated_bytes_after: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawPayloadField {
    Request,
    Response,
}

impl RawPayloadField {
    fn label(self) -> &'static str {
        match self {
            Self::Request => "request_raw_path",
            Self::Response => "response_raw_path",
        }
    }

    fn path_column(self) -> &'static str {
        self.label()
    }

    fn codec_column(self) -> &'static str {
        match self {
            Self::Request => "request_raw_codec",
            Self::Response => "response_raw_codec",
        }
    }
}

#[derive(Debug, Default)]
struct RawCompressionFileOutcome {
    candidate_counted: bool,
    compressed: bool,
    bytes_before: u64,
    bytes_after: u64,
    estimated_bytes_after: u64,
    new_db_path: Option<String>,
    new_codec: Option<String>,
    old_exact_path: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct RawCompressionBacklogSnapshot {
    oldest_uncompressed_age_secs: u64,
    uncompressed_count: u64,
    uncompressed_bytes: u64,
    alert_level: RawCompressionAlertLevel,
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum RawCompressionAlertLevel {
    #[default]
    Ok,
    Warn,
    Critical,
}

#[allow(dead_code)]
#[derive(Debug, Default)]
struct ArchiveManifestRefreshSummary {
    pending_batches: usize,
    refreshed_batches: usize,
    account_rows_written: usize,
    missing_files: usize,
}

struct CountingWriter<W> {
    inner: W,
    bytes_written: u64,
}

impl<W> CountingWriter<W> {
    fn new(inner: W) -> Self {
        Self {
            inner,
            bytes_written: 0,
        }
    }

    fn bytes_written(&self) -> u64 {
        self.bytes_written
    }
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.bytes_written += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[derive(Debug, FromRow, Clone)]
struct TimestampedArchiveCandidate {
    id: i64,
    timestamp_value: String,
}

#[derive(Debug, FromRow)]
struct DryRunBatchCount {
    month_key: String,
    row_count: i64,
}

const CODEX_INVOCATIONS_ARCHIVE_COLUMNS: &str = "id, invoke_id, occurred_at, source, model, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens, cost, status, error_message, failure_kind, failure_class, is_actionable, payload, raw_response, cost_estimated, price_version, request_raw_path, request_raw_codec, request_raw_size, request_raw_truncated, request_raw_truncated_reason, response_raw_path, response_raw_codec, response_raw_size, response_raw_truncated, response_raw_truncated_reason, detail_level, detail_pruned_at, detail_prune_reason, t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, created_at";
const FORWARD_PROXY_ATTEMPTS_ARCHIVE_COLUMNS: &str =
    "id, proxy_key, occurred_at, is_success, latency_ms, failure_kind, is_probe";
const POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_COLUMNS: &str = "id, invoke_id, occurred_at, endpoint, route_mode, sticky_key, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, requester_ip, started_at, finished_at, status, phase, http_status, downstream_http_status, failure_kind, error_message, downstream_error_message, connect_latency_ms, first_byte_latency_ms, stream_latency_ms, upstream_request_id, compact_support_status, compact_support_reason, created_at";
const STATS_SOURCE_SNAPSHOTS_ARCHIVE_COLUMNS: &str = "id, source, period, stats_date, model, requests, input_tokens, output_tokens, cache_create_tokens, cache_read_tokens, all_tokens, cost_input, cost_output, cost_cache_write, cost_cache_read, cost_total, raw_response, captured_at, captured_at_epoch, created_at";
const CODEX_QUOTA_SNAPSHOTS_ARCHIVE_COLUMNS: &str = "id, captured_at, amount_limit, used_amount, remaining_amount, period, period_reset_time, expire_time, is_active, total_cost, total_requests, total_tokens, last_request_time, billing_type, remaining_count, used_count, sub_type_name";

const CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS archive_db.codex_invocations (
    id INTEGER PRIMARY KEY,
    invoke_id TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT 'xy',
    model TEXT,
    input_tokens INTEGER,
    output_tokens INTEGER,
    cache_input_tokens INTEGER,
    reasoning_tokens INTEGER,
    total_tokens INTEGER,
    cost REAL,
    status TEXT,
    error_message TEXT,
    failure_kind TEXT,
    failure_class TEXT,
    is_actionable INTEGER NOT NULL DEFAULT 0,
    payload TEXT,
    raw_response TEXT NOT NULL,
    cost_estimated INTEGER NOT NULL DEFAULT 0,
    price_version TEXT,
    request_raw_path TEXT,
    request_raw_codec TEXT NOT NULL DEFAULT 'identity',
    request_raw_size INTEGER,
    request_raw_truncated INTEGER NOT NULL DEFAULT 0,
    request_raw_truncated_reason TEXT,
    response_raw_path TEXT,
    response_raw_codec TEXT NOT NULL DEFAULT 'identity',
    response_raw_size INTEGER,
    response_raw_truncated INTEGER NOT NULL DEFAULT 0,
    response_raw_truncated_reason TEXT,
    detail_level TEXT NOT NULL DEFAULT 'full',
    detail_pruned_at TEXT,
    detail_prune_reason TEXT,
    t_total_ms REAL,
    t_req_read_ms REAL,
    t_req_parse_ms REAL,
    t_upstream_connect_ms REAL,
    t_upstream_ttfb_ms REAL,
    t_upstream_stream_ms REAL,
    t_resp_parse_ms REAL,
    t_persist_ms REAL,
    created_at TEXT NOT NULL,
    UNIQUE(invoke_id, occurred_at)
)
"#;

const FORWARD_PROXY_ATTEMPTS_ARCHIVE_CREATE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS archive_db.forward_proxy_attempts (
    id INTEGER PRIMARY KEY,
    proxy_key TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    is_success INTEGER NOT NULL,
    latency_ms REAL,
    failure_kind TEXT,
    is_probe INTEGER NOT NULL DEFAULT 0
)
"#;

const POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_CREATE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS archive_db.pool_upstream_request_attempts (
    id INTEGER PRIMARY KEY,
    invoke_id TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    endpoint TEXT NOT NULL,
    route_mode TEXT NOT NULL,
    sticky_key TEXT,
    upstream_account_id INTEGER,
    upstream_route_key TEXT,
    attempt_index INTEGER NOT NULL,
    distinct_account_index INTEGER NOT NULL,
    same_account_retry_index INTEGER NOT NULL,
    requester_ip TEXT,
    started_at TEXT,
    finished_at TEXT,
    status TEXT NOT NULL,
    phase TEXT,
    http_status INTEGER,
    downstream_http_status INTEGER,
    failure_kind TEXT,
    error_message TEXT,
    downstream_error_message TEXT,
    connect_latency_ms REAL,
    first_byte_latency_ms REAL,
    stream_latency_ms REAL,
    upstream_request_id TEXT,
    compact_support_status TEXT,
    compact_support_reason TEXT,
    created_at TEXT NOT NULL
)
"#;

const STATS_SOURCE_SNAPSHOTS_ARCHIVE_CREATE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS archive_db.stats_source_snapshots (
    id INTEGER PRIMARY KEY,
    source TEXT NOT NULL,
    period TEXT NOT NULL,
    stats_date TEXT NOT NULL,
    model TEXT,
    requests INTEGER NOT NULL,
    input_tokens INTEGER,
    output_tokens INTEGER,
    cache_create_tokens INTEGER,
    cache_read_tokens INTEGER,
    all_tokens INTEGER,
    cost_input REAL,
    cost_output REAL,
    cost_cache_write REAL,
    cost_cache_read REAL,
    cost_total REAL,
    raw_response TEXT,
    captured_at TEXT NOT NULL,
    captured_at_epoch INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(source, period, stats_date, model, captured_at_epoch)
)
"#;

const CODEX_QUOTA_SNAPSHOTS_ARCHIVE_CREATE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS archive_db.codex_quota_snapshots (
    id INTEGER PRIMARY KEY,
    captured_at TEXT NOT NULL,
    amount_limit REAL,
    used_amount REAL,
    remaining_amount REAL,
    period TEXT,
    period_reset_time TEXT,
    expire_time TEXT,
    is_active INTEGER,
    total_cost REAL,
    total_requests INTEGER,
    total_tokens INTEGER,
    last_request_time TEXT,
    billing_type TEXT,
    remaining_count INTEGER,
    used_count INTEGER,
    sub_type_name TEXT
)
"#;

fn archive_table_spec(dataset: &'static str) -> ArchiveTableSpec {
    match dataset {
        "codex_invocations" => ArchiveTableSpec {
            dataset,
            columns: CODEX_INVOCATIONS_ARCHIVE_COLUMNS,
            create_sql: CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL,
        },
        "forward_proxy_attempts" => ArchiveTableSpec {
            dataset,
            columns: FORWARD_PROXY_ATTEMPTS_ARCHIVE_COLUMNS,
            create_sql: FORWARD_PROXY_ATTEMPTS_ARCHIVE_CREATE_SQL,
        },
        "pool_upstream_request_attempts" => ArchiveTableSpec {
            dataset,
            columns: POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_COLUMNS,
            create_sql: POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_CREATE_SQL,
        },
        "stats_source_snapshots" => ArchiveTableSpec {
            dataset,
            columns: STATS_SOURCE_SNAPSHOTS_ARCHIVE_COLUMNS,
            create_sql: STATS_SOURCE_SNAPSHOTS_ARCHIVE_CREATE_SQL,
        },
        "codex_quota_snapshots" => ArchiveTableSpec {
            dataset,
            columns: CODEX_QUOTA_SNAPSHOTS_ARCHIVE_COLUMNS,
            create_sql: CODEX_QUOTA_SNAPSHOTS_ARCHIVE_CREATE_SQL,
        },
        other => panic!("unsupported archive dataset: {other}"),
    }
}

fn spawn_data_retention_maintenance(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if !state.config.retention_enabled {
            info!("data retention maintenance is disabled");
            cancel.cancelled().await;
            return;
        }

        if cancel.is_cancelled() {
            info!("data retention maintenance skipped because shutdown is already in progress");
            return;
        }
        if let Err(err) =
            run_data_retention_maintenance(&state.pool, &state.config, None, Some(&cancel)).await
        {
            warn!(error = %err, "failed to run retention maintenance at startup");
        }

        let mut ticker = interval(state.config.retention_interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        ticker.tick().await;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("data retention maintenance received shutdown");
                    break;
                }
                _ = ticker.tick() => {
                    if let Err(err) = run_data_retention_maintenance(
                        &state.pool,
                        &state.config,
                        None,
                        Some(&cancel),
                    ).await {
                        warn!(error = %err, "failed to run retention maintenance");
                    }
                }
            }
        }
    })
}

fn should_stop_data_retention_maintenance(shutdown: Option<&CancellationToken>) -> bool {
    let should_stop = shutdown.is_some_and(CancellationToken::is_cancelled);
    if should_stop {
        info!(
            "data retention maintenance stopped at a safe boundary because shutdown is in progress"
        );
    }
    should_stop
}

async fn run_data_retention_maintenance(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run_override: Option<bool>,
    shutdown: Option<&CancellationToken>,
) -> Result<RetentionRunSummary> {
    let dry_run = dry_run_override.unwrap_or(config.retention_dry_run);
    let mut summary = RetentionRunSummary {
        dry_run,
        ..RetentionRunSummary::default()
    };
    let raw_path_fallback_root = config.database_path.parent();

    if !dry_run {
        sync_hourly_rollups_from_live_tables(pool).await?;
        let janitor = cleanup_stale_archive_temp_files(config, false)?;
        if janitor.stale_temp_files_removed > 0 {
            info!(
                ?janitor,
                "archive temp janitor removed stale files before retention"
            );
        }
    }

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    let raw_compression =
        compress_cold_proxy_raw_payloads(pool, config, raw_path_fallback_root, dry_run).await?;
    summary.raw_files_compression_candidates += raw_compression.files_considered;
    summary.raw_files_compressed += raw_compression.files_compressed;
    summary.raw_bytes_before += raw_compression.bytes_before;
    summary.raw_bytes_after += raw_compression.bytes_after;
    summary.raw_bytes_after_estimated += raw_compression.estimated_bytes_after;
    if !dry_run {
        log_raw_compression_backlog_if_needed(pool, config).await?;
    }

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    let pruned =
        prune_old_invocation_details(pool, config, raw_path_fallback_root, dry_run).await?;
    summary.invocation_details_pruned += pruned.0;
    summary.archive_batches_touched += pruned.1;
    summary.raw_files_removed += pruned.2;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    let invocation_archive =
        archive_old_invocations(pool, config, raw_path_fallback_root, dry_run).await?;
    summary.invocation_rows_archived += invocation_archive.0;
    summary.archive_batches_touched += invocation_archive.1;
    summary.raw_files_removed += invocation_archive.2;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    let proxy_archive = archive_timestamped_dataset(
        pool,
        config,
        archive_table_spec("forward_proxy_attempts"),
        "SELECT id, occurred_at AS timestamp_value FROM forward_proxy_attempts WHERE occurred_at < ?1 ORDER BY occurred_at ASC, id ASC LIMIT ?2",
        shanghai_utc_cutoff_string(config.forward_proxy_attempts_retention_days),
        dry_run,
    )
    .await?;
    summary.forward_proxy_attempt_rows_archived += proxy_archive.0;
    summary.archive_batches_touched += proxy_archive.1;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    let pool_attempt_archive = archive_timestamped_dataset(
        pool,
        config,
        archive_table_spec("pool_upstream_request_attempts"),
        "SELECT id, occurred_at AS timestamp_value FROM pool_upstream_request_attempts WHERE occurred_at < ?1 ORDER BY occurred_at ASC, id ASC LIMIT ?2",
        shanghai_local_cutoff_string(config.pool_upstream_request_attempts_retention_days),
        dry_run,
    )
    .await?;
    summary.pool_upstream_request_attempt_rows_archived += pool_attempt_archive.0;
    summary.archive_batches_touched += pool_attempt_archive.1;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    let snapshot_archive = archive_timestamped_dataset(
        pool,
        config,
        archive_table_spec("stats_source_snapshots"),
        "SELECT id, captured_at AS timestamp_value FROM stats_source_snapshots WHERE captured_at < ?1 ORDER BY captured_at ASC, id ASC LIMIT ?2",
        shanghai_utc_cutoff_string(config.stats_source_snapshots_retention_days),
        dry_run,
    )
    .await?;
    summary.stats_source_snapshot_rows_archived += snapshot_archive.0;
    summary.archive_batches_touched += snapshot_archive.1;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    let quota_archive = compact_old_quota_snapshots(pool, config, dry_run).await?;
    summary.quota_snapshot_rows_archived += quota_archive.0;
    summary.archive_batches_touched += quota_archive.1;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    summary.orphan_raw_files_removed +=
        sweep_orphan_proxy_raw_files(pool, config, raw_path_fallback_root, dry_run).await?;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    let archive_ttl_cleanup = cleanup_expired_archive_batches(pool, config, dry_run).await?;
    summary.archive_batches_deleted += archive_ttl_cleanup;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    if !dry_run && summary.touched_anything() {
        run_best_effort_retention_pragma(
            pool,
            "PRAGMA wal_checkpoint(PASSIVE)",
            "retention wal checkpoint",
        )
        .await?;
        run_best_effort_retention_pragma(pool, "PRAGMA optimize", "retention optimize pragma")
            .await?;
    }

    info!(
        dry_run = summary.dry_run,
        ?summary,
        "data retention maintenance finished"
    );
    Ok(summary)
}

async fn run_best_effort_retention_pragma(
    pool: &Pool<Sqlite>,
    sql: &str,
    description: &'static str,
) -> Result<()> {
    match sqlx::query(sql)
        .execute(pool)
        .await
        .with_context(|| format!("failed to run {description}"))
    {
        Ok(_) => Ok(()),
        Err(err) if is_sqlite_lock_error(&err) => {
            warn!(error = %err, sql, "{description} skipped because the database is busy");
            Ok(())
        }
        Err(err) => Err(err),
    }
}

async fn compress_cold_proxy_raw_payloads(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
) -> Result<RawCompressionPassSummary> {
    compress_cold_proxy_raw_payloads_with_budget(
        pool,
        config,
        raw_path_fallback_root,
        dry_run,
        Some(config.retention_catchup_budget),
    )
    .await
}

async fn compress_cold_proxy_raw_payloads_with_budget(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
    catchup_budget: Option<Duration>,
) -> Result<RawCompressionPassSummary> {
    if config.proxy_raw_compression == RawCompressionCodec::None {
        return Ok(RawCompressionPassSummary::default());
    }

    let mut summary = RawCompressionPassSummary::default();
    let started_at = Instant::now();
    let batch_limit = if dry_run {
        i64::MAX as usize
    } else {
        config.retention_batch_rows
    };

    loop {
        let (request_summary, request_hit_batch_limit) = compress_cold_proxy_raw_payload_lane(
            pool,
            config,
            raw_path_fallback_root,
            dry_run,
            RawPayloadField::Request,
            batch_limit,
        )
        .await?;
        accumulate_raw_compression_summary(&mut summary, request_summary);

        let (response_summary, response_hit_batch_limit) = compress_cold_proxy_raw_payload_lane(
            pool,
            config,
            raw_path_fallback_root,
            dry_run,
            RawPayloadField::Response,
            batch_limit,
        )
        .await?;
        accumulate_raw_compression_summary(&mut summary, response_summary);

        if !request_hit_batch_limit && !response_hit_batch_limit {
            break;
        }
        if dry_run {
            break;
        }
        if let Some(limit) = catchup_budget
            && started_at.elapsed() >= limit
        {
            break;
        }
    }

    Ok(summary)
}

fn accumulate_raw_compression_summary(
    target: &mut RawCompressionPassSummary,
    next: RawCompressionPassSummary,
) {
    target.files_considered += next.files_considered;
    target.files_compressed += next.files_compressed;
    target.bytes_before += next.bytes_before;
    target.bytes_after += next.bytes_after;
    target.estimated_bytes_after += next.estimated_bytes_after;
}

async fn compress_cold_proxy_raw_payload_lane(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
    field: RawPayloadField,
    batch_limit: usize,
) -> Result<(RawCompressionPassSummary, bool)> {
    let cutoff = shanghai_local_cutoff_for_age_secs_string(config.proxy_raw_hot_secs);
    let prune_cutoff = shanghai_local_cutoff_string(config.invocation_success_full_days);
    let archive_cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let success_like_condition = invocation_status_is_success_like_sql("status", "error_message");
    let sql = format!(
        r#"
        SELECT id, occurred_at, {path_column} AS raw_path
        FROM codex_invocations
        WHERE occurred_at < ?1
          AND occurred_at >= ?2
          AND (
            NOT {success_like_condition}
            OR detail_level IS NULL
            OR detail_level != ?3
            OR occurred_at >= ?4
          )
          AND {path_column} IS NOT NULL
          AND {codec_column} = ?5
          AND (
            ?6 IS NULL
            OR occurred_at > ?6
            OR (occurred_at = ?6 AND id > ?7)
          )
        ORDER BY occurred_at ASC, id ASC
        LIMIT ?8
        "#,
        path_column = field.path_column(),
        codec_column = field.codec_column(),
        success_like_condition = success_like_condition,
    );

    let mut summary = RawCompressionPassSummary::default();
    let mut last_seen_occurred_at: Option<String> = None;
    let mut last_seen_id = 0_i64;

    while summary.files_considered < batch_limit {
        let remaining = (batch_limit - summary.files_considered) as i64;
        let candidates = sqlx::query_as::<_, InvocationRawCompressionFieldCandidate>(&sql)
            .bind(&cutoff)
            .bind(&archive_cutoff)
            .bind(DETAIL_LEVEL_FULL)
            .bind(&prune_cutoff)
            .bind(RAW_CODEC_IDENTITY)
            .bind(last_seen_occurred_at.as_deref())
            .bind(last_seen_id)
            .bind(remaining.max(1))
            .fetch_all(pool)
            .await?;

        if candidates.is_empty() {
            break;
        }

        for candidate in candidates {
            last_seen_occurred_at = Some(candidate.occurred_at.clone());
            last_seen_id = candidate.id;

            let outcome = match maybe_compress_proxy_raw_path(
                pool,
                candidate.id,
                field.label(),
                Some(candidate.raw_path.as_str()),
                config.proxy_raw_compression,
                raw_path_fallback_root,
                dry_run,
            )
            .await
            {
                Ok(outcome) => outcome,
                Err(err) => {
                    warn!(
                        invocation_id = candidate.id,
                        field = field.label(),
                        error = %err,
                        "failed to cold-compress raw payload file; continuing retention"
                    );
                    continue;
                }
            };

            let next_path = outcome
                .new_db_path
                .clone()
                .unwrap_or_else(|| candidate.raw_path.clone());
            let next_codec = outcome
                .new_codec
                .clone()
                .unwrap_or_else(|| raw_codec_from_path(Some(next_path.as_str())));

            if !dry_run
                && (next_path != candidate.raw_path || !raw_codec_is_identity(Some(&next_codec)))
            {
                let update_sql = format!(
                    "UPDATE codex_invocations SET {path_column} = ?1, {codec_column} = ?2 WHERE id = ?3",
                    path_column = field.path_column(),
                    codec_column = field.codec_column(),
                );
                sqlx::query(&update_sql)
                    .bind(&next_path)
                    .bind(&next_codec)
                    .bind(candidate.id)
                    .execute(pool)
                    .await?;

                if let Some(path) = outcome.old_exact_path.as_deref()
                    && next_path != candidate.raw_path
                {
                    delete_exact_proxy_raw_path(Some(path), raw_path_fallback_root)?;
                }
            }

            if outcome.candidate_counted {
                summary.files_considered += 1;
            }
            if outcome.compressed {
                summary.files_compressed += 1;
            }
            summary.bytes_before += outcome.bytes_before;
            summary.bytes_after += outcome.bytes_after;
            summary.estimated_bytes_after += outcome.estimated_bytes_after;

            if summary.files_considered >= batch_limit {
                break;
            }
        }
    }

    let hit_batch_limit = summary.files_considered >= batch_limit;
    Ok((summary, hit_batch_limit))
}

async fn maybe_compress_proxy_raw_path(
    _pool: &Pool<Sqlite>,
    invocation_id: i64,
    field_name: &str,
    raw_path: Option<&str>,
    codec: RawCompressionCodec,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
) -> Result<RawCompressionFileOutcome> {
    let Some(raw_path) = raw_path else {
        return Ok(RawCompressionFileOutcome::default());
    };
    if codec == RawCompressionCodec::None || raw_path.ends_with(".gz") {
        return Ok(RawCompressionFileOutcome {
            new_db_path: Some(raw_path.to_string()),
            new_codec: Some(RAW_CODEC_GZIP.to_string()),
            ..RawCompressionFileOutcome::default()
        });
    }

    let Some(source_path) = locate_existing_proxy_raw_path(raw_path, raw_path_fallback_root) else {
        let existing_compressed =
            locate_existing_proxy_raw_compressed_path(raw_path, raw_path_fallback_root);
        if existing_compressed.is_some() {
            return Ok(RawCompressionFileOutcome {
                new_db_path: Some(raw_payload_compressed_db_path(raw_path)),
                new_codec: Some(RAW_CODEC_GZIP.to_string()),
                ..RawCompressionFileOutcome::default()
            });
        }
        warn!(
            invocation_id,
            field = field_name,
            raw_path,
            "skipping raw cold compression because source raw file is missing"
        );
        return Ok(RawCompressionFileOutcome {
            new_db_path: Some(raw_path.to_string()),
            new_codec: Some(raw_codec_from_path(Some(raw_path))),
            ..RawCompressionFileOutcome::default()
        });
    };

    let source_meta = fs::metadata(&source_path).with_context(|| {
        format!(
            "failed to inspect raw payload before cold compression: {}",
            source_path.display()
        )
    })?;
    if !source_meta.is_file() {
        return Ok(RawCompressionFileOutcome {
            new_db_path: Some(raw_path.to_string()),
            new_codec: Some(raw_codec_from_path(Some(raw_path))),
            ..RawCompressionFileOutcome::default()
        });
    }

    let target_db_path = raw_payload_compressed_db_path(raw_path);
    let target_path = raw_payload_compressed_file_path(&source_path);
    let bytes_before = source_meta.len();
    if dry_run {
        let estimated_bytes_after = estimate_gzip_file_size(&source_path)?;
        return Ok(RawCompressionFileOutcome {
            candidate_counted: true,
            bytes_before,
            estimated_bytes_after,
            new_db_path: Some(target_db_path),
            new_codec: Some(RAW_CODEC_GZIP.to_string()),
            old_exact_path: Some(source_path),
            ..RawCompressionFileOutcome::default()
        });
    }

    let bytes_after = compress_file_to_gzip(&source_path, &target_path)?;
    Ok(RawCompressionFileOutcome {
        candidate_counted: true,
        compressed: true,
        bytes_before,
        bytes_after,
        new_db_path: Some(target_db_path),
        new_codec: Some(RAW_CODEC_GZIP.to_string()),
        old_exact_path: Some(source_path),
        ..RawCompressionFileOutcome::default()
    })
}

fn compress_file_to_gzip(source: &Path, destination: &Path) -> Result<u64> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create raw compression directory {}",
                parent.display()
            )
        })?;
    }

    let temp_destination = PathBuf::from(format!("{}.tmp", destination.display()));
    if temp_destination.exists() {
        let _ = fs::remove_file(&temp_destination);
    }

    let result = (|| -> Result<u64> {
        let input = fs::File::open(source)
            .with_context(|| format!("failed to open raw payload {}", source.display()))?;
        let output = fs::File::create(&temp_destination).with_context(|| {
            format!(
                "failed to create compressed raw payload {}",
                temp_destination.display()
            )
        })?;
        let mut reader = io::BufReader::new(input);
        let counting_writer = CountingWriter::new(io::BufWriter::new(output));
        let mut encoder = GzEncoder::new(counting_writer, Compression::default());
        io::copy(&mut reader, &mut encoder).with_context(|| {
            format!(
                "failed to compress raw payload {} into {}",
                source.display(),
                temp_destination.display()
            )
        })?;
        let mut counting_writer = encoder.finish().with_context(|| {
            format!(
                "failed to finish raw payload compression {}",
                temp_destination.display()
            )
        })?;
        counting_writer.flush()?;
        let bytes_after = counting_writer.bytes_written();
        let mut output = counting_writer.inner;
        output.flush()?;
        fs::rename(&temp_destination, destination).with_context(|| {
            format!(
                "failed to move compressed raw payload into place: {} -> {}",
                temp_destination.display(),
                destination.display()
            )
        })?;
        Ok(bytes_after)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp_destination);
    }
    result
}

fn estimate_gzip_file_size(source: &Path) -> Result<u64> {
    let input = fs::File::open(source)
        .with_context(|| format!("failed to open raw payload {}", source.display()))?;
    let mut reader = io::BufReader::new(input);
    let counting_writer = CountingWriter::new(io::sink());
    let mut encoder = GzEncoder::new(counting_writer, Compression::default());
    io::copy(&mut reader, &mut encoder).with_context(|| {
        format!(
            "failed to estimate gzip size for raw payload {}",
            source.display()
        )
    })?;
    let counting_writer = encoder.finish().with_context(|| {
        format!(
            "failed to finish gzip size estimate for raw payload {}",
            source.display()
        )
    })?;
    Ok(counting_writer.bytes_written())
}

fn raw_payload_compressed_db_path(raw_path: &str) -> String {
    if raw_path.ends_with(".gz") {
        raw_path.to_string()
    } else {
        format!("{raw_path}.gz")
    }
}

fn raw_codec_from_path(raw_path: Option<&str>) -> String {
    match raw_path {
        Some(path) if path.ends_with(".gz") => RAW_CODEC_GZIP.to_string(),
        _ => RAW_CODEC_IDENTITY.to_string(),
    }
}

fn raw_codec_is_identity(raw_codec: Option<&str>) -> bool {
    matches!(raw_codec, Some(RAW_CODEC_IDENTITY) | None)
}

fn raw_payload_compressed_file_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.gz", path.display()))
}

fn locate_existing_proxy_raw_path(path: &str, fallback_root: Option<&Path>) -> Option<PathBuf> {
    resolved_raw_path_candidates(path, fallback_root)
        .into_iter()
        .find(|candidate| candidate.exists())
}

fn locate_existing_proxy_raw_compressed_path(
    path: &str,
    fallback_root: Option<&Path>,
) -> Option<PathBuf> {
    resolved_raw_path_candidates(&raw_payload_compressed_db_path(path), fallback_root)
        .into_iter()
        .find(|candidate| candidate.exists())
}

fn delete_exact_proxy_raw_path(
    raw_path: Option<&Path>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<()> {
    let Some(raw_path) = raw_path else {
        return Ok(());
    };
    let raw_path = raw_path.to_string_lossy();
    for candidate in resolved_raw_path_candidates(&raw_path, raw_path_fallback_root) {
        match fs::remove_file(&candidate) {
            Ok(_) => return Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => {
                warn!(
                    path = %candidate.display(),
                    error = %err,
                    "failed to remove replaced raw payload after cold compression"
                );
                return Ok(());
            }
        }
    }
    Ok(())
}

async fn prune_old_invocation_details(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
) -> Result<(usize, usize, usize)> {
    let prune_cutoff = shanghai_local_cutoff_string(config.invocation_success_full_days);
    let archive_cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let spec = archive_table_spec("codex_invocations");
    let success_like_condition = invocation_status_is_success_like_sql("status", "error_message");
    if dry_run {
        let sql = format!(
            r#"
            SELECT id, occurred_at, request_raw_path, response_raw_path
            FROM codex_invocations
            WHERE {success_like_condition}
              AND detail_level = ?1
              AND occurred_at < ?2
              AND occurred_at >= ?3
            ORDER BY occurred_at ASC, id ASC
            "#,
            success_like_condition = success_like_condition,
        );
        let candidates = sqlx::query_as::<_, InvocationDetailPruneCandidate>(&sql)
            .bind(DETAIL_LEVEL_FULL)
            .bind(&prune_cutoff)
            .bind(&archive_cutoff)
            .fetch_all(pool)
            .await?;
        let mut by_group: BTreeMap<String, usize> = BTreeMap::new();
        for candidate in &candidates {
            let group_key = invocation_archive_group_key(config, &candidate.occurred_at)?;
            *by_group.entry(group_key).or_default() += 1;
        }
        for (group_key, rows) in &by_group {
            info!(
                dataset = spec.dataset,
                archive_group = group_key,
                rows = *rows,
                reason = DETAIL_PRUNE_REASON_SUCCESS_OVER_30D,
                "retention dry-run planned invocation detail prune archive batch"
            );
        }
        let raw_paths = candidates
            .iter()
            .flat_map(|candidate| {
                [
                    candidate.request_raw_path.clone(),
                    candidate.response_raw_path.clone(),
                ]
            })
            .collect::<Vec<_>>();
        return Ok((
            candidates.len(),
            by_group.len(),
            count_existing_proxy_raw_paths(&raw_paths, raw_path_fallback_root),
        ));
    }

    let mut rows_pruned = 0usize;
    let mut archive_batches = 0usize;
    let mut raw_files_removed = 0usize;

    loop {
        let sql = format!(
            r#"
            SELECT id, occurred_at, request_raw_path, response_raw_path
            FROM codex_invocations
            WHERE {success_like_condition}
              AND detail_level = ?1
              AND occurred_at < ?2
              AND occurred_at >= ?3
            ORDER BY occurred_at ASC, id ASC
            LIMIT ?4
            "#,
            success_like_condition = success_like_condition,
        );
        let candidates = sqlx::query_as::<_, InvocationDetailPruneCandidate>(&sql)
            .bind(DETAIL_LEVEL_FULL)
            .bind(&prune_cutoff)
            .bind(&archive_cutoff)
            .bind(config.retention_batch_rows as i64)
            .fetch_all(pool)
            .await?;

        if candidates.is_empty() {
            break;
        }

        let mut by_group: BTreeMap<String, Vec<InvocationDetailPruneCandidate>> = BTreeMap::new();
        for candidate in candidates {
            let group_key = invocation_archive_group_key(config, &candidate.occurred_at)?;
            by_group.entry(group_key).or_default().push(candidate);
        }

        for (group_key, group) in by_group {
            rows_pruned += group.len();
            archive_batches += 1;
            let raw_paths = group
                .iter()
                .flat_map(|candidate| {
                    [
                        candidate.request_raw_path.clone(),
                        candidate.response_raw_path.clone(),
                    ]
                })
                .collect::<Vec<_>>();

            let ids = group
                .iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            let mut archive_outcome = match archive_layout_for_dataset(config, spec.dataset) {
                ArchiveBatchLayout::LegacyMonth => {
                    archive_rows_into_month_batch(pool, config, spec, &group_key, &ids).await?
                }
                ArchiveBatchLayout::SegmentV1 => {
                    archive_rows_into_segment_batch(pool, config, spec, &group_key, &ids).await?
                }
            };
            set_archive_batch_coverage_from_local_rows(
                &mut archive_outcome,
                group.iter().map(|candidate| candidate.occurred_at.as_str()),
                Some(config.invocation_archive_ttl_days),
            )?;
            let pruned_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
            let mut tx = pool.begin().await?;
            upsert_archive_batch_manifest(tx.as_mut(), &archive_outcome).await?;
            mark_archive_batch_historical_rollups_materialized_tx(
                tx.as_mut(),
                spec.dataset,
                &archive_outcome.file_path,
            )
            .await?;
            let mut query = QueryBuilder::<Sqlite>::new(
                "UPDATE codex_invocations SET payload = CASE WHEN json_valid(payload) AND json_extract(payload, '$.upstreamAccountId') IS NOT NULL THEN json_object('upstreamAccountId', json_extract(payload, '$.upstreamAccountId')) ELSE NULL END, raw_response = '', request_raw_path = NULL, request_raw_codec = 'identity', request_raw_size = NULL, request_raw_truncated = 0, request_raw_truncated_reason = NULL, response_raw_path = NULL, response_raw_codec = 'identity', response_raw_size = NULL, response_raw_truncated = 0, response_raw_truncated_reason = NULL, detail_level = ",
            );
            query
                .push_bind(DETAIL_LEVEL_STRUCTURED_ONLY)
                .push(", detail_pruned_at = ")
                .push_bind(pruned_at)
                .push(", detail_prune_reason = ")
                .push_bind(DETAIL_PRUNE_REASON_SUCCESS_OVER_30D)
                .push(" WHERE id IN (");
            {
                let mut separated = query.separated(", ");
                for id in &ids {
                    separated.push_bind(id);
                }
            }
            query.push(")");
            query.build().execute(tx.as_mut()).await?;
            tx.commit().await?;

            raw_files_removed += delete_proxy_raw_paths(&raw_paths, raw_path_fallback_root)?;
        }
    }

    Ok((rows_pruned, archive_batches, raw_files_removed))
}

async fn archive_old_invocations(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
) -> Result<(usize, usize, usize)> {
    let cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let spec = archive_table_spec("codex_invocations");

    if dry_run {
        let candidates = sqlx::query_as::<_, InvocationArchiveCandidate>(
            r#"
            SELECT
                id,
                occurred_at,
                source,
                status,
                total_tokens,
                cost,
                request_raw_path,
                response_raw_path
            FROM codex_invocations
            WHERE occurred_at < ?1
            ORDER BY occurred_at ASC, id ASC
            "#,
        )
        .bind(&cutoff)
        .fetch_all(pool)
        .await?;

        let mut by_group: BTreeMap<String, usize> = BTreeMap::new();
        for candidate in &candidates {
            let group_key = invocation_archive_group_key(config, &candidate.occurred_at)?;
            *by_group.entry(group_key).or_default() += 1;
        }
        for (group_key, rows) in &by_group {
            info!(
                dataset = spec.dataset,
                archive_group = group_key,
                rows = *rows,
                reason = DETAIL_PRUNE_REASON_MAX_AGE_ARCHIVED,
                "retention dry-run planned invocation archive batch"
            );
        }
        let raw_paths = candidates
            .iter()
            .flat_map(|candidate| {
                [
                    candidate.request_raw_path.clone(),
                    candidate.response_raw_path.clone(),
                ]
            })
            .collect::<Vec<_>>();
        return Ok((
            candidates.len(),
            by_group.len(),
            count_existing_proxy_raw_paths(&raw_paths, raw_path_fallback_root),
        ));
    }

    let mut rows_archived = 0usize;
    let mut archive_batches = 0usize;
    let mut raw_files_removed = 0usize;

    loop {
        let candidates = sqlx::query_as::<_, InvocationArchiveCandidate>(
            r#"
            SELECT
                id,
                occurred_at,
                source,
                status,
                total_tokens,
                cost,
                request_raw_path,
                response_raw_path
            FROM codex_invocations
            WHERE occurred_at < ?1
            ORDER BY occurred_at ASC, id ASC
            LIMIT ?2
            "#,
        )
        .bind(&cutoff)
        .bind(config.retention_batch_rows as i64)
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        let mut by_group: BTreeMap<String, Vec<InvocationArchiveCandidate>> = BTreeMap::new();
        for candidate in candidates {
            let group_key = invocation_archive_group_key(config, &candidate.occurred_at)?;
            by_group.entry(group_key).or_default().push(candidate);
        }

        for (group_key, group) in by_group {
            rows_archived += group.len();
            archive_batches += 1;
            let raw_paths = group
                .iter()
                .flat_map(|candidate| {
                    [
                        candidate.request_raw_path.clone(),
                        candidate.response_raw_path.clone(),
                    ]
                })
                .collect::<Vec<_>>();

            let ids = group
                .iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            let materialized_rows = group
                .iter()
                .map(|candidate| InvocationHourlySourceRecord {
                    id: candidate.id,
                    occurred_at: candidate.occurred_at.clone(),
                    source: candidate.source.clone(),
                    status: candidate.status.clone(),
                    detail_level: DETAIL_LEVEL_FULL.to_string(),
                    total_tokens: None,
                    cost: None,
                    error_message: None,
                    failure_kind: None,
                    failure_class: None,
                    is_actionable: None,
                    payload: None,
                    t_total_ms: None,
                    t_req_read_ms: None,
                    t_req_parse_ms: None,
                    t_upstream_connect_ms: None,
                    t_upstream_ttfb_ms: None,
                    t_upstream_stream_ms: None,
                    t_resp_parse_ms: None,
                    t_persist_ms: None,
                })
                .collect::<Vec<_>>();
            let mut archive_outcome = match archive_layout_for_dataset(config, spec.dataset) {
                ArchiveBatchLayout::LegacyMonth => {
                    archive_rows_into_month_batch(pool, config, spec, &group_key, &ids).await?
                }
                ArchiveBatchLayout::SegmentV1 => {
                    archive_rows_into_segment_batch(pool, config, spec, &group_key, &ids).await?
                }
            };
            set_archive_batch_coverage_from_local_rows(
                &mut archive_outcome,
                group.iter().map(|candidate| candidate.occurred_at.as_str()),
                None,
            )?;
            archive_outcome.archive_expires_at =
                Some(shanghai_archive_expiry_from_reference_timestamp(
                    &format_utc_iso(Utc::now()),
                    config.invocation_archive_ttl_days,
                )?);
            let mut tx = pool.begin().await?;
            upsert_invocation_rollups(tx.as_mut(), &group).await?;
            upsert_archive_batch_manifest(tx.as_mut(), &archive_outcome).await?;
            mark_archive_batch_historical_rollups_materialized_tx(
                tx.as_mut(),
                spec.dataset,
                &archive_outcome.file_path,
            )
            .await?;
            delete_rows_by_ids(tx.as_mut(), spec.dataset, &ids).await?;
            mark_retention_archived_hourly_rollup_targets_tx(
                tx.as_mut(),
                spec.dataset,
                &materialized_rows,
                &[],
            )
            .await?;
            tx.commit().await?;
            raw_files_removed += delete_proxy_raw_paths(&raw_paths, raw_path_fallback_root)?;
        }
    }

    Ok((rows_archived, archive_batches, raw_files_removed))
}

async fn archive_timestamped_dataset(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    spec: ArchiveTableSpec,
    select_sql: &str,
    cutoff: String,
    dry_run: bool,
) -> Result<(usize, usize)> {
    if dry_run {
        let dry_run_sql = match spec.dataset {
            "forward_proxy_attempts" => {
                r#"
                SELECT strftime('%Y-%m', datetime(occurred_at, '+8 hours')) AS month_key,
                       COUNT(*) AS row_count
                FROM forward_proxy_attempts
                WHERE occurred_at < ?1
                GROUP BY 1
                ORDER BY 1
                "#
            }
            "pool_upstream_request_attempts" => {
                r#"
                SELECT strftime('%Y-%m', occurred_at) AS month_key,
                       COUNT(*) AS row_count
                FROM pool_upstream_request_attempts
                WHERE occurred_at < ?1
                GROUP BY 1
                ORDER BY 1
                "#
            }
            "stats_source_snapshots" => {
                r#"
                SELECT strftime('%Y-%m', datetime(captured_at, '+8 hours')) AS month_key,
                       COUNT(*) AS row_count
                FROM stats_source_snapshots
                WHERE captured_at < ?1
                GROUP BY 1
                ORDER BY 1
                "#
            }
            other => bail!("unsupported dry-run archive dataset: {other}"),
        };
        let batch_counts = sqlx::query_as::<_, DryRunBatchCount>(dry_run_sql)
            .bind(&cutoff)
            .fetch_all(pool)
            .await?;
        for batch in &batch_counts {
            info!(
                dataset = spec.dataset,
                month_key = %batch.month_key,
                rows = batch.row_count,
                "retention dry-run planned archive batch"
            );
        }
        return Ok((
            batch_counts
                .iter()
                .map(|batch| batch.row_count as usize)
                .sum(),
            batch_counts.len(),
        ));
    }

    let mut rows_archived = 0usize;
    let mut archive_batches = 0usize;

    loop {
        let candidates = sqlx::query_as::<_, TimestampedArchiveCandidate>(select_sql)
            .bind(&cutoff)
            .bind(config.retention_batch_rows as i64)
            .fetch_all(pool)
            .await?;

        if candidates.is_empty() {
            break;
        }

        let mut by_month: BTreeMap<String, Vec<TimestampedArchiveCandidate>> = BTreeMap::new();
        for candidate in candidates {
            let month_key =
                archive_timestamped_dataset_month_key(spec.dataset, &candidate.timestamp_value)?;
            by_month.entry(month_key).or_default().push(candidate);
        }

        for (month_key, group) in by_month {
            rows_archived += group.len();
            archive_batches += 1;
            let ids = group
                .iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            let materialized_forward_proxy_rows = if spec.dataset == "forward_proxy_attempts" {
                group
                    .iter()
                    .map(|candidate| ForwardProxyAttemptHourlySourceRecord {
                        id: candidate.id,
                        proxy_key: String::new(),
                        occurred_at: candidate.timestamp_value.clone(),
                        is_success: 0,
                        latency_ms: None,
                    })
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            let mut archive_outcome =
                archive_rows_into_month_batch(pool, config, spec, &month_key, &ids).await?;
            if spec.dataset == "pool_upstream_request_attempts" {
                set_archive_batch_coverage_from_local_rows(
                    &mut archive_outcome,
                    group
                        .iter()
                        .map(|candidate| candidate.timestamp_value.as_str()),
                    Some(config.pool_upstream_request_attempts_archive_ttl_days),
                )?;
            } else {
                set_archive_batch_coverage_from_utc_rows(
                    &mut archive_outcome,
                    group
                        .iter()
                        .map(|candidate| candidate.timestamp_value.as_str()),
                )?;
            }
            let mut tx = pool.begin().await?;
            upsert_archive_batch_manifest(tx.as_mut(), &archive_outcome).await?;
            mark_archive_batch_historical_rollups_materialized_tx(
                tx.as_mut(),
                spec.dataset,
                &archive_outcome.file_path,
            )
            .await?;
            delete_rows_by_ids(tx.as_mut(), spec.dataset, &ids).await?;
            mark_retention_archived_hourly_rollup_targets_tx(
                tx.as_mut(),
                spec.dataset,
                &[],
                &materialized_forward_proxy_rows,
            )
            .await?;
            tx.commit().await?;
        }
    }

    Ok((rows_archived, archive_batches))
}

fn archive_timestamped_dataset_month_key(dataset: &str, timestamp_value: &str) -> Result<String> {
    match dataset {
        "pool_upstream_request_attempts" => shanghai_month_key_from_local_naive(timestamp_value),
        _ => shanghai_month_key_from_utc_naive(timestamp_value),
    }
}

fn set_archive_batch_coverage_from_local_rows<'a>(
    batch: &mut ArchiveBatchOutcome,
    rows: impl Iterator<Item = &'a str>,
    archive_ttl_days: Option<u64>,
) -> Result<()> {
    let values = rows.collect::<Vec<_>>();
    if values.is_empty() {
        return Ok(());
    }
    let mut sorted = values.into_iter().map(str::to_string).collect::<Vec<_>>();
    sorted.sort();
    batch.coverage_start_at = sorted.first().cloned();
    batch.coverage_end_at = sorted.last().cloned();
    batch.archive_expires_at = match (batch.coverage_end_at.as_deref(), archive_ttl_days) {
        (Some(coverage_end_at), Some(ttl_days)) => Some(
            shanghai_archive_expiry_from_local_timestamp(coverage_end_at, ttl_days)?,
        ),
        _ => None,
    };
    Ok(())
}

fn set_archive_batch_coverage_from_utc_rows<'a>(
    batch: &mut ArchiveBatchOutcome,
    rows: impl Iterator<Item = &'a str>,
) -> Result<()> {
    let values = rows.collect::<Vec<_>>();
    if values.is_empty() {
        return Ok(());
    }
    let mut sorted = values.into_iter().map(str::to_string).collect::<Vec<_>>();
    sorted.sort();
    batch.coverage_start_at = sorted.first().cloned();
    batch.coverage_end_at = sorted.last().cloned();
    batch.archive_expires_at = None;
    Ok(())
}

fn shanghai_archive_expiry_from_local_timestamp(
    value: &str,
    archive_ttl_days: u64,
) -> Result<String> {
    let local = parse_shanghai_local_naive(value)?;
    shanghai_archive_expiry_from_local_naive(local, archive_ttl_days)
}

fn shanghai_archive_expiry_from_reference_timestamp(
    value: &str,
    archive_ttl_days: u64,
) -> Result<String> {
    let local = match parse_to_utc_datetime(value) {
        Some(value) => value.with_timezone(&Shanghai).naive_local(),
        None => parse_shanghai_local_naive(value)?,
    };
    shanghai_archive_expiry_from_local_naive(local, archive_ttl_days)
}

fn shanghai_archive_expiry_from_local_naive(
    local: NaiveDateTime,
    archive_ttl_days: u64,
) -> Result<String> {
    let expiry = start_of_local_day(local_naive_to_utc(local, Shanghai), Shanghai)
        + ChronoDuration::days(archive_ttl_days as i64 + 1);
    Ok(format_naive(expiry.with_timezone(&Shanghai).naive_local()))
}

#[derive(Debug, FromRow)]
struct ArchiveExpiryBackfillCandidate {
    id: i64,
    coverage_end_at: String,
}
