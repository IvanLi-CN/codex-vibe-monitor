use super::*;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ArchiveTableSpec {
    pub(crate) dataset: &'static str,
    pub(crate) columns: &'static str,
    pub(crate) create_sql: &'static str,
}

#[derive(Debug, Clone)]
pub(crate) struct ArchiveBatchOutcome {
    pub(crate) dataset: &'static str,
    pub(crate) month_key: String,
    pub(crate) day_key: Option<String>,
    pub(crate) part_key: Option<String>,
    pub(crate) file_path: String,
    pub(crate) sha256: String,
    pub(crate) source_identity_sha256: Option<String>,
    pub(crate) row_count: i64,
    pub(crate) upstream_last_activity: Vec<(i64, String)>,
    pub(crate) coverage_start_at: Option<String>,
    pub(crate) coverage_end_at: Option<String>,
    pub(crate) archive_expires_at: Option<String>,
    pub(crate) summary_source_kind: &'static str,
    pub(crate) layout: &'static str,
    pub(crate) codec: &'static str,
    pub(crate) writer_version: &'static str,
    pub(crate) cleanup_state: &'static str,
    pub(crate) superseded_by: Option<i64>,
}

#[derive(Debug, Default)]
pub(crate) struct InvocationRollupDelta {
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
}

#[derive(Debug, FromRow)]
pub(crate) struct InvocationDetailPruneCandidate {
    pub(crate) id: i64,
    pub(crate) occurred_at: String,
    pub(crate) request_raw_path: Option<String>,
    pub(crate) response_raw_path: Option<String>,
    pub(crate) estimated_write_bytes: i64,
}

#[derive(Debug, FromRow, Clone)]
pub(crate) struct InvocationArchiveCandidate {
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) source: String,
    pub(crate) status: Option<String>,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    #[sqlx(default)]
    pub(crate) reasoning_tokens: Option<i64>,
    pub(crate) total_tokens: Option<i64>,
    pub(crate) cost: Option<f64>,
    pub(crate) first_token_ms: Option<f64>,
    pub(crate) payload: Option<String>,
    pub(crate) request_raw_path: Option<String>,
    pub(crate) response_raw_path: Option<String>,
}

pub(crate) async fn load_invocation_archive_candidates_by_ids(
    pool: &Pool<Sqlite>,
    ids: &[i64],
) -> Result<Vec<InvocationArchiveCandidate>> {
    let ids_json = serde_json::to_string(ids)?;
    sqlx::query_as::<_, InvocationArchiveCandidate>(
        r#"
        SELECT id, invoke_id, occurred_at, source, status, input_tokens, output_tokens,
               cache_input_tokens, reasoning_tokens, total_tokens, cost, first_token_ms,
               payload, request_raw_path, response_raw_path
        FROM codex_invocations
        WHERE id IN (SELECT value FROM json_each(?1))
        ORDER BY id ASC
        "#,
    )
    .bind(ids_json)
    .fetch_all(pool)
    .await
    .context("failed to load invocation archive candidates")
}

#[derive(Debug, FromRow, Clone)]
pub(crate) struct InvocationRawCompressionFieldCandidate {
    pub(crate) id: i64,
    pub(crate) occurred_at: String,
    pub(crate) raw_path: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct RawPathReferenceCandidate {
    pub(crate) reference_kind: String,
    pub(crate) id: i64,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct ArchiveBatchFileRow {
    pub(crate) id: i64,
    pub(crate) file_path: String,
    pub(crate) sha256: Option<String>,
    pub(crate) coverage_start_at: Option<String>,
    pub(crate) coverage_end_at: Option<String>,
}

#[derive(Debug, FromRow)]
pub(crate) struct ArchiveManifestBatchRow {
    pub(crate) id: i64,
    pub(crate) file_path: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct ArchiveStorageManifestRow {
    pub(crate) id: i64,
    pub(crate) dataset: String,
    pub(crate) layout: String,
    pub(crate) file_path: String,
}

#[derive(Debug, Default)]
pub(crate) struct ArchiveTempCleanupSummary {
    pub(crate) stale_temp_files_removed: usize,
    pub(crate) stale_temp_bytes_removed: u64,
}

#[derive(Debug, Default)]
pub(crate) struct ArchiveStorageVerificationSummary {
    pub(crate) manifest_rows: usize,
    pub(crate) missing_files: usize,
    pub(crate) orphan_files: usize,
    pub(crate) stale_temp_files: usize,
    pub(crate) stale_temp_bytes: u64,
}

#[derive(Debug, Default)]
pub(crate) struct ArchiveBatchPruneSummary {
    pub(crate) expired_archive_batches_deleted: usize,
    pub(crate) legacy_archive_batches_deleted: usize,
}

#[derive(Debug, FromRow)]
pub(crate) struct RawCompressionBacklogAggRow {
    pub(crate) uncompressed_count: i64,
    pub(crate) uncompressed_bytes: Option<i64>,
    pub(crate) oldest_occurred_at: Option<String>,
}

#[derive(Debug, FromRow)]
pub(crate) struct ArchivedAccountLastActivityRow {
    pub(crate) account_id: i64,
    pub(crate) last_activity_at: String,
}

pub(crate) fn dedupe_archive_upstream_last_activity(
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
pub(crate) struct ArchiveBackfillSummary {
    pub(crate) scanned_batches: u64,
    pub(crate) updated_accounts: u64,
    pub(crate) hit_budget: bool,
    pub(crate) waiting_for_manifest_backfill: bool,
}

#[allow(dead_code)]
#[derive(Debug, Default)]
pub(crate) struct HistoricalRollupMaterializationSummary {
    pub(crate) scanned_archive_batches: usize,
    pub(crate) skipped_archive_batches: usize,
    pub(crate) materialized_archive_batches: usize,
    pub(crate) blocked_archive_batches: usize,
    pub(crate) materialized_bucket_count: usize,
    pub(crate) materialized_invocation_batches: usize,
    pub(crate) materialized_forward_proxy_batches: usize,
    pub(crate) last_materialized_bucket_start_epoch: Option<i64>,
}

#[allow(dead_code)]
#[derive(Debug, Default)]
pub(crate) struct LegacyArchivePruneSummary {
    pub(crate) scanned_archive_batches: usize,
    pub(crate) deleted_archive_batches: usize,
    pub(crate) skipped_unmaterialized_batches: usize,
    pub(crate) skipped_retained_batches: usize,
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
    pub(crate) pending_usage_breakdown_batches: u64,
    pub(crate) last_materialized_hour: Option<String>,
    pub(crate) alert_level: HistoricalRollupBackfillAlertLevel,
}

pub(crate) const HOURLY_ROLLUP_DATASET_INVOCATIONS: &str = "codex_invocations";
pub(crate) const SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE: &str = "authoritative";
pub(crate) const SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR: &str = "live_mirror";
pub(crate) const SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN: &str = "unknown";
pub(crate) const HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS: &str = "forward_proxy_attempts";
pub(crate) const HOURLY_ROLLUP_DATASET_UPSTREAM_HOST_NETWORK_DIRECT: &str =
    "upstream_host_network_direct";
pub(crate) const HOURLY_ROLLUP_DATASET_UPSTREAM_HOST_NETWORK_POOL_ATTEMPTS: &str =
    "upstream_host_network_pool_attempts";
pub(crate) const HOURLY_ROLLUP_TARGET_INVOCATIONS: &str = "invocation_rollup_hourly";
pub(crate) const HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES: &str =
    "invocation_failure_rollup_hourly";
pub(crate) const HOURLY_ROLLUP_TARGET_PROXY_PERF: &str = "proxy_perf_stage_hourly";
pub(crate) const HOURLY_ROLLUP_TARGET_PROMPT_CACHE: &str = "prompt_cache_rollup_hourly";
pub(crate) const HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS: &str =
    "prompt_cache_upstream_account_hourly";
pub(crate) const HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE: &str =
    "upstream_account_usage_hourly";
pub(crate) const HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN: &str =
    "upstream_account_usage_breakdown_hourly";
pub(crate) const HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY: &str =
    "upstream_account_stats_hourly";
pub(crate) const HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2: &str =
    "upstream_account_activity_hourly_v2";
pub(crate) const HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE: &str =
    "upstream_account_stats_minute";
pub(crate) const HOURLY_ROLLUP_TARGET_UPSTREAM_HOST_NETWORK_MINUTE: &str =
    "upstream_host_network_minute";
pub(crate) const HOURLY_ROLLUP_TARGET_STICKY_KEYS: &str = "upstream_sticky_key_hourly";
pub(crate) const HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS: &str = "forward_proxy_attempt_hourly";
pub(crate) const HISTORICAL_ROLLUP_ARCHIVE_DATASETS: [&str; 2] = [
    HOURLY_ROLLUP_DATASET_INVOCATIONS,
    HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
];
pub(crate) const INVOCATION_HOURLY_ROLLUP_TARGETS: [&str; 11] = [
    HOURLY_ROLLUP_TARGET_INVOCATIONS,
    HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
    HOURLY_ROLLUP_TARGET_PROXY_PERF,
    HOURLY_ROLLUP_TARGET_PROMPT_CACHE,
    HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS,
    HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE,
    HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
    HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
    HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2,
    HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE,
    HOURLY_ROLLUP_TARGET_STICKY_KEYS,
];
pub(crate) const SUMMARY_PROJECTION_ARCHIVE_REPLAY_TARGETS: [&str; 3] = [
    HOURLY_ROLLUP_TARGET_INVOCATIONS,
    HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
    HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
];
pub(crate) const PERF_STAGE_TOTAL: &str = "total";
pub(crate) const PERF_STAGE_REQUEST_READ: &str = "requestRead";
pub(crate) const PERF_STAGE_REQUEST_PARSE: &str = "requestParse";
pub(crate) const PERF_STAGE_UPSTREAM_CONNECT: &str = "upstreamConnect";
pub(crate) const PERF_STAGE_UPSTREAM_FIRST_BYTE: &str = "upstreamFirstByte";
pub(crate) const PERF_STAGE_UPSTREAM_STREAM: &str = "upstreamStream";
pub(crate) const PERF_STAGE_RESPONSE_PARSE: &str = "responseParse";
pub(crate) const PERF_STAGE_PERSISTENCE: &str = "persistence";
pub(crate) const HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE: &str = "";
pub(crate) const UPSTREAM_ACCOUNT_ACTIVITY_UNASSIGNED_ID: i64 = -1;

#[derive(Debug, Clone, FromRow)]
pub(crate) struct InvocationHourlySourceRecord {
    pub(crate) id: i64,
    pub(crate) occurred_at: String,
    pub(crate) source: String,
    pub(crate) status: Option<String>,
    pub(crate) detail_level: String,
    #[sqlx(default)]
    pub(crate) model: Option<String>,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    #[sqlx(default)]
    pub(crate) reasoning_tokens: Option<i64>,
    pub(crate) total_tokens: Option<i64>,
    pub(crate) cost: Option<f64>,
    #[sqlx(default)]
    pub(crate) upstream_account_id: Option<i64>,
    #[sqlx(default)]
    pub(crate) cost_input: Option<f64>,
    #[sqlx(default)]
    pub(crate) cost_cache_write: Option<f64>,
    #[sqlx(default)]
    pub(crate) cost_cache_read: Option<f64>,
    #[sqlx(default)]
    pub(crate) cost_output: Option<f64>,
    #[sqlx(default)]
    pub(crate) cost_reasoning: Option<f64>,
    pub(crate) error_message: Option<String>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) failure_class: Option<String>,
    pub(crate) is_actionable: Option<i64>,
    pub(crate) payload: Option<String>,
    pub(crate) t_total_ms: Option<f64>,
    pub(crate) t_req_read_ms: Option<f64>,
    pub(crate) t_req_parse_ms: Option<f64>,
    pub(crate) t_upstream_connect_ms: Option<f64>,
    pub(crate) t_upstream_ttfb_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) first_token_ms: Option<f64>,
    pub(crate) t_upstream_stream_ms: Option<f64>,
    pub(crate) t_resp_parse_ms: Option<f64>,
    pub(crate) t_persist_ms: Option<f64>,
}

impl InvocationHourlySourceRecord {
    pub(crate) fn resolved_upstream_account_id(&self) -> Option<i64> {
        self.upstream_account_id
            .or_else(|| crate::proxy::upstream_account_id_from_payload(self.payload.as_deref()))
    }

    pub(crate) fn has_complete_token_components(&self) -> bool {
        self.total_tokens.unwrap_or_default() <= 0
            || ((self.input_tokens.is_some()
                && self.output_tokens.is_some()
                && self.cache_input_tokens.is_some()
                && self.reasoning_tokens.is_some())
                || (self.input_tokens.is_none()
                    && self.output_tokens.is_none()
                    && self.cache_input_tokens.is_none()
                    && self.reasoning_tokens.is_none()))
    }

    pub(crate) fn has_known_token_components(&self) -> bool {
        self.total_tokens.unwrap_or_default() <= 0
            || (self.input_tokens.is_some()
                && self.output_tokens.is_some()
                && self.cache_input_tokens.is_some()
                && self.reasoning_tokens.is_some())
    }
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct ForwardProxyAttemptHourlySourceRecord {
    pub(crate) id: i64,
    pub(crate) proxy_key: String,
    pub(crate) occurred_at: String,
    pub(crate) is_success: i64,
    pub(crate) latency_ms: Option<f64>,
}

#[derive(Debug)]
pub(crate) struct TempSqliteCleanup {
    pub(crate) path: PathBuf,
    pub(crate) reusable_before_run: bool,
    pub(crate) keep: bool,
}

impl TempSqliteCleanup {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self::with_reusable_copy(path, false)
    }

    pub(crate) fn with_reusable_copy(path: PathBuf, reusable_before_run: bool) -> Self {
        Self {
            path,
            reusable_before_run,
            keep: false,
        }
    }

    pub(crate) fn keep_complete_copy(&mut self) {
        self.keep = true;
    }

    pub(crate) fn keep_reusable_copy(&mut self) {
        if self.reusable_before_run {
            self.keep = true;
        }
    }
}

pub(crate) fn temp_sqlite_source_meta_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.source-meta", path.display()))
}

pub(crate) fn remove_temp_sqlite_artifacts(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(temp_sqlite_source_meta_path(path));
    for suffix in ["-journal", "-wal", "-shm"] {
        let _ = fs::remove_file(PathBuf::from(format!("{}{suffix}", path.display())));
    }
}

impl Drop for TempSqliteCleanup {
    fn drop(&mut self) {
        if !self.keep {
            remove_temp_sqlite_artifacts(&self.path);
        }
    }
}

pub(crate) fn sqlite_url_for_path(path: &Path) -> String {
    format!("sqlite://{}", path.to_string_lossy())
}

#[derive(Debug, Default)]
pub(crate) struct RawCompressionPassSummary {
    pub(crate) files_considered: usize,
    pub(crate) files_compressed: usize,
    pub(crate) bytes_before: u64,
    pub(crate) bytes_after: u64,
    pub(crate) estimated_bytes_after: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RawPayloadField {
    Request,
    Response,
}

impl RawPayloadField {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Request => "request_raw_path",
            Self::Response => "response_raw_path",
        }
    }

    pub(crate) fn path_column(self) -> &'static str {
        self.label()
    }

    pub(crate) fn codec_column(self) -> &'static str {
        match self {
            Self::Request => "request_raw_codec",
            Self::Response => "response_raw_codec",
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct RawCompressionFileOutcome {
    pub(crate) candidate_counted: bool,
    pub(crate) compressed: bool,
    pub(crate) bytes_before: u64,
    pub(crate) bytes_after: u64,
    pub(crate) estimated_bytes_after: u64,
    pub(crate) new_db_path: Option<String>,
    pub(crate) new_codec: Option<String>,
    pub(crate) old_exact_path: Option<PathBuf>,
}

#[derive(Debug, Default)]
pub(crate) struct RawCompressionBacklogSnapshot {
    pub(crate) oldest_uncompressed_age_secs: u64,
    pub(crate) uncompressed_count: u64,
    pub(crate) uncompressed_bytes: u64,
    pub(crate) alert_level: RawCompressionAlertLevel,
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RawCompressionAlertLevel {
    #[default]
    Ok,
    Warn,
    Critical,
}

#[allow(dead_code)]
#[derive(Debug, Default)]
pub(crate) struct ArchiveManifestRefreshSummary {
    pub(crate) pending_batches: usize,
    pub(crate) candidate_remaining_hint: usize,
    pub(crate) refreshed_batches: usize,
    pub(crate) account_rows_written: usize,
    pub(crate) missing_files: usize,
}

pub(crate) struct CountingWriter<W> {
    pub(crate) inner: W,
    pub(crate) bytes_written: u64,
}

impl<W> CountingWriter<W> {
    pub(crate) fn new(inner: W) -> Self {
        Self {
            inner,
            bytes_written: 0,
        }
    }

    pub(crate) fn bytes_written(&self) -> u64 {
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
pub(crate) struct TimestampedArchiveCandidate {
    pub(crate) id: i64,
    pub(crate) timestamp_value: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct DryRunBatchCount {
    pub(crate) month_key: String,
    pub(crate) row_count: i64,
}

pub(crate) const CODEX_INVOCATIONS_ARCHIVE_COLUMNS: &str = "id, invoke_id, occurred_at, source, model, input_tokens, output_tokens, cache_input_tokens, reported_cache_write_tokens, reasoning_tokens, total_tokens, cost, cost_input, cost_cache_write, cost_cache_read, cost_output, cost_reasoning, status, error_message, failure_kind, failure_class, is_actionable, payload, raw_response, cost_estimated, price_version, request_raw_path, request_raw_codec, request_raw_size, request_raw_truncated, request_raw_truncated_reason, response_raw_path, response_raw_codec, response_raw_size, response_raw_truncated, response_raw_truncated_reason, detail_level, detail_pruned_at, detail_prune_reason, t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, first_token_ms, t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, created_at";
pub(crate) const FORWARD_PROXY_ATTEMPTS_ARCHIVE_COLUMNS: &str =
    "id, proxy_key, occurred_at, is_success, latency_ms, failure_kind, is_probe";
pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_COLUMNS: &str = "id, attempt_public_id, invoke_id, occurred_at, endpoint, route_mode, sticky_key, routing_source, routing_selection_audit_json, upstream_base_url_host, group_name_snapshot, proxy_binding_key_snapshot, request_model, upstream_request_model, model_mapping_pattern, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, requester_ip, started_at, finished_at, status, phase, http_status, downstream_http_status, failure_kind, error_message, downstream_error_message, connect_latency_ms, first_byte_latency_ms, stream_latency_ms, upstream_request_id, upstream_request_compression_algorithm, upstream_request_compression_mode, upstream_request_logical_body_bytes, upstream_request_transmitted_body_bytes, upstream_request_header_bytes_approx, upstream_response_body_bytes, upstream_response_header_bytes_approx, compact_support_status, compact_support_reason, request_summary_json, response_summary_json, response_raw_path, response_raw_codec, response_raw_size, response_raw_truncated, response_raw_truncated_reason, response_content_encoding, created_at";
pub(crate) const CODEX_QUOTA_SNAPSHOTS_ARCHIVE_COLUMNS: &str = "id, captured_at, amount_limit, used_amount, remaining_amount, period, period_reset_time, expire_time, is_active, total_cost, total_requests, total_tokens, last_request_time, billing_type, remaining_count, used_count, sub_type_name";

pub(crate) const CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS archive_db.codex_invocations (
    id INTEGER PRIMARY KEY,
    invoke_id TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT 'xy',
    model TEXT,
    input_tokens INTEGER,
    output_tokens INTEGER,
    cache_input_tokens INTEGER,
    reported_cache_write_tokens INTEGER,
    reasoning_tokens INTEGER,
    total_tokens INTEGER,
    cost REAL,
    cost_input REAL,
    cost_cache_write REAL,
    cost_cache_read REAL,
    cost_output REAL,
    cost_reasoning REAL,
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
    first_token_ms REAL,
    t_upstream_stream_ms REAL,
    t_resp_parse_ms REAL,
    t_persist_ms REAL,
    created_at TEXT NOT NULL,
    UNIQUE(invoke_id, occurred_at)
)
"#;

pub(crate) const FORWARD_PROXY_ATTEMPTS_ARCHIVE_CREATE_SQL: &str = r#"
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

pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_CREATE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS archive_db.pool_upstream_request_attempts (
    id INTEGER PRIMARY KEY,
    attempt_public_id TEXT,
    invoke_id TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    endpoint TEXT NOT NULL,
    route_mode TEXT NOT NULL,
    sticky_key TEXT,
    routing_source TEXT,
    routing_selection_audit_json TEXT,
    upstream_base_url_host TEXT,
    group_name_snapshot TEXT,
    proxy_binding_key_snapshot TEXT,
    request_model TEXT,
    upstream_request_model TEXT,
    model_mapping_pattern TEXT,
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
    upstream_request_compression_algorithm TEXT,
    upstream_request_compression_mode TEXT,
    upstream_request_logical_body_bytes INTEGER,
    upstream_request_transmitted_body_bytes INTEGER,
    upstream_request_header_bytes_approx INTEGER,
    upstream_response_body_bytes INTEGER,
    upstream_response_header_bytes_approx INTEGER,
    compact_support_status TEXT,
    compact_support_reason TEXT,
    request_summary_json TEXT,
    response_summary_json TEXT,
    response_raw_path TEXT,
    response_raw_codec TEXT NOT NULL DEFAULT 'identity',
    response_raw_size INTEGER,
    response_raw_truncated INTEGER NOT NULL DEFAULT 0,
    response_raw_truncated_reason TEXT,
    response_content_encoding TEXT,
    created_at TEXT NOT NULL
)
"#;

pub(crate) const CODEX_QUOTA_SNAPSHOTS_ARCHIVE_CREATE_SQL: &str = r#"
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

pub(crate) fn archive_table_spec(dataset: &'static str) -> ArchiveTableSpec {
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
        "codex_quota_snapshots" => ArchiveTableSpec {
            dataset,
            columns: CODEX_QUOTA_SNAPSHOTS_ARCHIVE_COLUMNS,
            create_sql: CODEX_QUOTA_SNAPSHOTS_ARCHIVE_CREATE_SQL,
        },
        other => panic!("unsupported archive dataset: {other}"),
    }
}

pub(crate) fn archive_timestamped_dataset_month_key(
    dataset: &str,
    timestamp_value: &str,
) -> Result<String> {
    match dataset {
        "pool_upstream_request_attempts" => shanghai_month_key_from_local_naive(timestamp_value),
        _ => shanghai_month_key_from_utc_naive(timestamp_value),
    }
}

pub(crate) fn set_archive_batch_coverage_from_local_rows<'a>(
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

pub(crate) async fn pool_upstream_month_archive_reappeared_after_cleanup(
    pool: &Pool<Sqlite>,
    archive_file_path: &str,
) -> Result<bool> {
    let existing_manifest_rows: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM archive_batches
        WHERE dataset = 'pool_upstream_request_attempts'
          AND file_path = ?1
        "#,
    )
    .bind(archive_file_path)
    .fetch_one(pool)
    .await?;
    if existing_manifest_rows > 0 {
        return Ok(false);
    }

    let existing_hourly_rows: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_node_health_hourly_archive
        WHERE archive_file_path = ?1
        "#,
    )
    .bind(archive_file_path)
    .fetch_one(pool)
    .await?;

    Ok(existing_hourly_rows > 0)
}

pub(crate) fn set_archive_batch_coverage_from_utc_rows<'a>(
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

pub(crate) fn shanghai_archive_expiry_from_local_timestamp(
    value: &str,
    archive_ttl_days: u64,
) -> Result<String> {
    let local = parse_shanghai_local_naive(value)?;
    shanghai_archive_expiry_from_local_naive(local, archive_ttl_days)
}

pub(crate) fn shanghai_archive_expiry_from_reference_timestamp(
    value: &str,
    archive_ttl_days: u64,
) -> Result<String> {
    let local = match parse_to_utc_datetime(value) {
        Some(value) => value.with_timezone(&Shanghai).naive_local(),
        None => parse_shanghai_local_naive(value)?,
    };
    shanghai_archive_expiry_from_local_naive(local, archive_ttl_days)
}

pub(crate) fn shanghai_archive_expiry_from_local_naive(
    local: NaiveDateTime,
    archive_ttl_days: u64,
) -> Result<String> {
    let expiry = start_of_local_day(local_naive_to_utc(local, Shanghai), Shanghai)
        + ChronoDuration::days(archive_ttl_days as i64 + 1);
    Ok(format_naive(expiry.with_timezone(&Shanghai).naive_local()))
}

#[derive(Debug, FromRow)]
pub(crate) struct ArchiveExpiryBackfillCandidate {
    pub(crate) id: i64,
    pub(crate) coverage_end_at: String,
}
