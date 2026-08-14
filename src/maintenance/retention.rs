use super::*;

const RETENTION_FAIRNESS_INTERVAL: Duration = Duration::from_secs(15);
const RETENTION_WRITE_TARGET: Duration = Duration::from_millis(200);
const RETENTION_WRITE_WARNING: Duration = Duration::from_millis(250);
const RETENTION_WRITE_INITIAL_ROWS: usize = 4;
pub(super) const RETENTION_WRITE_MAX_ROWS: usize = 64;
const RETENTION_WRITE_MAX_BYTES: usize = 1024 * 1024;

tokio::task_local! {
    static RETENTION_SHUTDOWN: CancellationToken;
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RetentionWriteHealthSnapshot {
    pub(crate) state: String,
    pub(crate) operation: Option<String>,
    pub(crate) admission_mode: Option<String>,
    pub(crate) batch_rows: usize,
    pub(crate) estimated_bytes: usize,
    pub(crate) prepare_elapsed_ms: u64,
    pub(crate) lock_wait_ms: u64,
    pub(crate) execute_ms: u64,
    pub(crate) commit_ms: u64,
    pub(crate) budget_breach_count: u64,
    pub(crate) defer_reason: Option<String>,
    pub(crate) starvation_age_ms: Option<u64>,
    pub(crate) p1_waiter_count: usize,
    pub(crate) candidate_remaining_hint: usize,
    pub(crate) last_error: Option<String>,
}

impl Default for RetentionWriteHealthSnapshot {
    fn default() -> Self {
        Self {
            state: "healthy".to_string(),
            operation: None,
            admission_mode: None,
            batch_rows: 0,
            estimated_bytes: 0,
            prepare_elapsed_ms: 0,
            lock_wait_ms: 0,
            execute_ms: 0,
            commit_ms: 0,
            budget_breach_count: 0,
            defer_reason: None,
            starvation_age_ms: None,
            p1_waiter_count: 0,
            candidate_remaining_hint: 0,
            last_error: None,
        }
    }
}

#[derive(Debug)]
struct RetentionWriteBudget {
    next_rows: usize,
    estimated_bytes_per_row: usize,
}

impl Default for RetentionWriteBudget {
    fn default() -> Self {
        Self {
            next_rows: RETENTION_WRITE_INITIAL_ROWS,
            estimated_bytes_per_row: 256,
        }
    }
}

impl RetentionWriteBudget {
    fn candidate_limit(&self, configured_limit: usize) -> usize {
        let byte_limited_rows = RETENTION_WRITE_MAX_BYTES
            .checked_div(self.estimated_bytes_per_row.max(1))
            .unwrap_or(1)
            .max(1);
        self.next_rows
            .min(byte_limited_rows)
            .min(RETENTION_WRITE_MAX_ROWS)
            .min(configured_limit.max(1))
            .max(1)
    }

    fn observe_commit(&mut self, rows: usize, estimated_bytes: usize, elapsed: Duration) -> bool {
        let observed_bytes_per_row = estimated_bytes.saturating_div(rows.max(1)).max(1);
        self.estimated_bytes_per_row = self
            .estimated_bytes_per_row
            .saturating_mul(3)
            .saturating_add(observed_bytes_per_row)
            .saturating_div(4)
            .max(1);
        let breached =
            elapsed > RETENTION_WRITE_WARNING || estimated_bytes > RETENTION_WRITE_MAX_BYTES;
        if breached {
            self.next_rows = self.next_rows.saturating_div(2).max(1);
        } else if elapsed <= RETENTION_WRITE_TARGET && estimated_bytes < RETENTION_WRITE_MAX_BYTES {
            self.next_rows = self
                .next_rows
                .saturating_add(1)
                .min(RETENTION_WRITE_MAX_ROWS);
        }
        breached
    }
}

#[derive(Debug, Default)]
struct RetentionWriteHealthState {
    snapshot: RetentionWriteHealthSnapshot,
    budgets: HashMap<&'static str, RetentionWriteBudget>,
}

static RETENTION_WRITE_HEALTH: Lazy<std::sync::Mutex<RetentionWriteHealthState>> =
    Lazy::new(|| std::sync::Mutex::new(RetentionWriteHealthState::default()));

#[derive(Debug)]
pub(super) struct RetentionWriteDeferred {
    operation: &'static str,
}

impl std::fmt::Display for RetentionWriteDeferred {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "retention write deferred before {}",
            self.operation
        )
    }
}

impl std::error::Error for RetentionWriteDeferred {}

pub(super) fn retention_write_deferred(operation: &'static str) -> anyhow::Error {
    anyhow::Error::new(RetentionWriteDeferred { operation })
}

pub(super) fn is_retention_write_deferred(error: &anyhow::Error) -> bool {
    error.is::<RetentionWriteDeferred>()
}

pub(super) fn retention_prepared_batch_or_deferred<T>(result: Result<T>) -> Result<Option<T>> {
    match result {
        Err(error) if is_retention_write_deferred(&error) => Ok(None),
        Ok(value) => Ok(Some(value)),
        Err(error) => Err(error),
    }
}

pub(crate) fn retention_write_health_snapshot() -> RetentionWriteHealthSnapshot {
    RETENTION_WRITE_HEALTH
        .lock()
        .expect("retention write health")
        .snapshot
        .clone()
}

pub(super) fn retention_candidate_limit(config: &AppConfig, operation: &'static str) -> usize {
    if cfg!(test) {
        return config.retention_batch_rows;
    }
    RETENTION_WRITE_HEALTH
        .lock()
        .expect("retention write health")
        .budgets
        .entry(operation)
        .or_default()
        .candidate_limit(config.retention_batch_rows)
}

pub(super) fn retention_micro_batch_limit(config: &AppConfig, operation: &'static str) -> usize {
    retention_candidate_limit(config, operation).min(RETENTION_WRITE_MAX_ROWS)
}

fn retention_record_defer(operation: &'static str, reason: impl ToString) {
    let reason = reason.to_string();
    let mut health = RETENTION_WRITE_HEALTH
        .lock()
        .expect("retention write health");
    health.snapshot.state = "deferred".to_string();
    health.snapshot.operation = Some(operation.to_string());
    health.snapshot.defer_reason = Some(reason.clone());
    health.snapshot.last_error = None;
    debug!(
        operation,
        defer_reason = %reason,
        "retention write deferred before SQLite admission"
    );
}

pub(crate) struct RetentionWriteCommit {
    pub(crate) operation: &'static str,
    pub(crate) admission_mode: &'static str,
    pub(crate) rows: usize,
    pub(crate) estimated_bytes: usize,
    pub(crate) prepare_elapsed: Duration,
    pub(crate) lock_wait: Duration,
    pub(crate) execute_elapsed: Duration,
    pub(crate) commit_elapsed: Duration,
    pub(crate) p1_waiter_count: usize,
    pub(crate) candidate_remaining_hint: usize,
}

macro_rules! retention_record_commit {
    (
        $operation:expr,
        $admission_mode:expr,
        $rows:expr,
        $estimated_bytes:expr,
        $prepare_elapsed:expr,
        $lock_wait:expr,
        $execute_elapsed:expr,
        $commit_elapsed:expr,
        $p1_waiter_count:expr,
        $candidate_remaining_hint:expr $(,)?
    ) => {
        $crate::maintenance::retention::record_retention_write_commit(
            $crate::maintenance::retention::RetentionWriteCommit {
                operation: $operation,
                admission_mode: $admission_mode,
                rows: $rows,
                estimated_bytes: $estimated_bytes,
                prepare_elapsed: $prepare_elapsed,
                lock_wait: $lock_wait,
                execute_elapsed: $execute_elapsed,
                commit_elapsed: $commit_elapsed,
                p1_waiter_count: $p1_waiter_count,
                candidate_remaining_hint: $candidate_remaining_hint,
            },
        )
    };
}

pub(crate) use retention_record_commit;

pub(crate) fn record_retention_write_commit(commit: RetentionWriteCommit) {
    let RetentionWriteCommit {
        operation,
        admission_mode,
        rows,
        estimated_bytes,
        prepare_elapsed,
        lock_wait,
        execute_elapsed,
        commit_elapsed,
        p1_waiter_count,
        candidate_remaining_hint,
    } = commit;
    let elapsed = execute_elapsed.saturating_add(commit_elapsed);
    let mut health = RETENTION_WRITE_HEALTH
        .lock()
        .expect("retention write health");
    let breached =
        health
            .budgets
            .entry(operation)
            .or_default()
            .observe_commit(rows, estimated_bytes, elapsed);
    if breached {
        health.snapshot.budget_breach_count = health.snapshot.budget_breach_count.saturating_add(1);
        health.snapshot.state = "degraded".to_string();
    } else {
        health.snapshot.state = "healthy".to_string();
    }
    health.snapshot.operation = Some(operation.to_string());
    health.snapshot.admission_mode = Some(admission_mode.to_string());
    health.snapshot.batch_rows = rows;
    health.snapshot.estimated_bytes = estimated_bytes;
    health.snapshot.prepare_elapsed_ms = prepare_elapsed.as_millis() as u64;
    health.snapshot.lock_wait_ms = lock_wait.as_millis() as u64;
    health.snapshot.execute_ms = execute_elapsed.as_millis() as u64;
    health.snapshot.commit_ms = commit_elapsed.as_millis() as u64;
    health.snapshot.defer_reason = None;
    health.snapshot.starvation_age_ms = if admission_mode == "fairness" {
        Some(lock_wait.as_millis() as u64)
    } else {
        None
    };
    health.snapshot.p1_waiter_count = p1_waiter_count;
    health.snapshot.candidate_remaining_hint = candidate_remaining_hint;
    health.snapshot.last_error = None;
    if breached {
        warn!(
            operation,
            admission_mode,
            batch_rows = rows,
            estimated_bytes,
            prepare_elapsed_ms = prepare_elapsed.as_millis() as u64,
            lock_wait_ms = lock_wait.as_millis() as u64,
            execute_ms = execute_elapsed.as_millis() as u64,
            commit_ms = commit_elapsed.as_millis() as u64,
            p1_waiter_count,
            candidate_remaining_hint,
            "retention write transaction exceeded its micro-batch budget"
        );
    } else {
        debug!(
            operation,
            admission_mode,
            batch_rows = rows,
            estimated_bytes,
            prepare_elapsed_ms = prepare_elapsed.as_millis() as u64,
            lock_wait_ms = lock_wait.as_millis() as u64,
            execute_ms = execute_elapsed.as_millis() as u64,
            p1_waiter_count,
            candidate_remaining_hint,
            "retention write micro-batch committed"
        );
    }
}

fn retention_record_error(operation: &'static str, error: &anyhow::Error) {
    let mut health = RETENTION_WRITE_HEALTH
        .lock()
        .expect("retention write health");
    health.snapshot.state = "degraded".to_string();
    health.snapshot.operation = Some(operation.to_string());
    health.snapshot.last_error = Some(error.to_string());
}

pub(super) fn take_retention_micro_batch<T>(
    candidates: Vec<T>,
    estimated_bytes: impl Fn(&T) -> usize,
) -> Vec<T> {
    let mut selected = Vec::new();
    let mut total_bytes = 0usize;
    for candidate in candidates {
        let row_bytes = estimated_bytes(&candidate).max(1);
        if !selected.is_empty()
            && (selected.len() >= RETENTION_WRITE_MAX_ROWS
                || total_bytes.saturating_add(row_bytes) > RETENTION_WRITE_MAX_BYTES)
        {
            break;
        }
        total_bytes = total_bytes.saturating_add(row_bytes);
        selected.push(candidate);
        if selected.len() >= RETENTION_WRITE_MAX_ROWS {
            break;
        }
    }
    selected
}

pub(super) struct RetentionWriteAdmission {
    write_permit: crate::proxy_sqlite_write_coordinator::ProxySqliteWritePermit,
    _pressure_permit: crate::db_pressure::DbBackgroundPermit,
    p1_waiter_count: usize,
}

impl RetentionWriteAdmission {
    pub(super) fn admission_mode(&self) -> &'static str {
        if self.write_permit.fairness_admission() {
            "fairness"
        } else {
            "normal"
        }
    }

    pub(super) fn lock_wait(&self) -> Duration {
        self.write_permit.lock_wait()
    }

    pub(super) fn p1_waiter_count(&self) -> usize {
        self.p1_waiter_count
    }
}

pub(super) async fn acquire_retention_write_admission(
    operation: &'static str,
) -> Option<RetentionWriteAdmission> {
    let pressure_gate = crate::db_pressure::global_db_pressure_gate();
    if let Some(reason) = pressure_gate.background_deny_reason() {
        retention_record_defer(operation, reason);
        return None;
    }
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let write_permit = match RETENTION_SHUTDOWN.try_with(Clone::clone) {
        Ok(shutdown) => {
            coordinator
                .acquire_maintenance_cancellable(RETENTION_FAIRNESS_INTERVAL, &shutdown)
                .await
        }
        Err(_) => Some(
            coordinator
                .acquire_maintenance(RETENTION_FAIRNESS_INTERVAL)
                .await,
        ),
    };
    let Some(mut write_permit) = write_permit else {
        retention_record_defer(operation, "shutdown");
        return None;
    };
    let coordinator_snapshot =
        crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .snapshot()
            .await;
    match pressure_gate.try_begin_background(operation) {
        Ok(pressure_permit) => Some(RetentionWriteAdmission {
            write_permit,
            _pressure_permit: pressure_permit,
            p1_waiter_count: coordinator_snapshot.p1_waiter_count,
        }),
        Err(reason) => {
            retention_record_defer(operation, reason);
            write_permit.revoke_fairness_admission();
            drop(write_permit);
            None
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct RetentionRunSummary {
    pub(crate) dry_run: bool,
    pub(crate) raw_files_compression_candidates: usize,
    pub(crate) raw_files_compressed: usize,
    pub(crate) raw_bytes_before: u64,
    pub(crate) raw_bytes_after: u64,
    pub(crate) raw_bytes_after_estimated: u64,
    pub(crate) invocation_details_pruned: usize,
    pub(crate) invocation_rows_archived: usize,
    pub(crate) forward_proxy_attempt_rows_archived: usize,
    pub(crate) pool_upstream_request_attempt_rows_archived: usize,
    pub(crate) quota_snapshot_rows_archived: usize,
    pub(crate) archive_batches_touched: usize,
    pub(crate) archive_batches_deleted: usize,
    pub(crate) raw_files_removed: usize,
    pub(crate) orphan_raw_files_removed: usize,
    pub(crate) model_route_rows_pruned: usize,
}

impl RetentionRunSummary {
    fn touched_anything(&self) -> bool {
        self.raw_files_compression_candidates > 0
            || self.raw_files_compressed > 0
            || self.invocation_details_pruned > 0
            || self.invocation_rows_archived > 0
            || self.forward_proxy_attempt_rows_archived > 0
            || self.pool_upstream_request_attempt_rows_archived > 0
            || self.quota_snapshot_rows_archived > 0
            || self.archive_batches_deleted > 0
            || self.raw_files_removed > 0
            || self.orphan_raw_files_removed > 0
            || self.model_route_rows_pruned > 0
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ArchiveTableSpec {
    pub(crate) dataset: &'static str,
    pub(crate) columns: &'static str,
    pub(crate) create_sql: &'static str,
}

#[derive(Debug)]
pub(crate) struct ArchiveBatchOutcome {
    pub(crate) dataset: &'static str,
    pub(crate) month_key: String,
    pub(crate) day_key: Option<String>,
    pub(crate) part_key: Option<String>,
    pub(crate) file_path: String,
    pub(crate) sha256: String,
    pub(crate) row_count: i64,
    pub(crate) upstream_last_activity: Vec<(i64, String)>,
    pub(crate) coverage_start_at: Option<String>,
    pub(crate) coverage_end_at: Option<String>,
    pub(crate) archive_expires_at: Option<String>,
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
    pub(crate) occurred_at: String,
    pub(crate) source: String,
    pub(crate) status: Option<String>,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) total_tokens: Option<i64>,
    pub(crate) cost: Option<f64>,
    pub(crate) first_token_ms: Option<f64>,
    pub(crate) payload: Option<String>,
    pub(crate) request_raw_path: Option<String>,
    pub(crate) response_raw_path: Option<String>,
}

fn invocation_archive_candidate_to_hourly_source_record(
    candidate: &InvocationArchiveCandidate,
) -> InvocationHourlySourceRecord {
    InvocationHourlySourceRecord {
        id: candidate.id,
        occurred_at: candidate.occurred_at.clone(),
        source: candidate.source.clone(),
        status: candidate.status.clone(),
        detail_level: DETAIL_LEVEL_FULL.to_string(),
        model: None,
        input_tokens: candidate.input_tokens,
        output_tokens: candidate.output_tokens,
        cache_input_tokens: candidate.cache_input_tokens,
        total_tokens: candidate.total_tokens,
        cost: candidate.cost,
        upstream_account_id: None,
        cost_input: None,
        cost_cache_write: None,
        cost_cache_read: None,
        cost_output: None,
        cost_reasoning: None,
        error_message: None,
        failure_kind: None,
        failure_class: None,
        is_actionable: None,
        payload: candidate.payload.clone(),
        t_total_ms: None,
        t_req_read_ms: None,
        t_req_parse_ms: None,
        t_upstream_connect_ms: None,
        t_upstream_ttfb_ms: None,
        first_token_ms: candidate.first_token_ms,
        t_upstream_stream_ms: None,
        t_resp_parse_ms: None,
        t_persist_ms: None,
    }
}

#[cfg(test)]
mod ttft_retention_tests {
    use super::*;

    #[test]
    fn archived_invocation_keeps_ttft_for_rollup_materialization() {
        let candidate = InvocationArchiveCandidate {
            id: 7,
            occurred_at: "2026-07-25 12:00:00".to_string(),
            source: SOURCE_PROXY.to_string(),
            status: Some("success".to_string()),
            input_tokens: Some(10),
            output_tokens: Some(2),
            cache_input_tokens: Some(3),
            total_tokens: Some(12),
            cost: Some(0.01),
            first_token_ms: Some(321.0),
            payload: None,
            request_raw_path: None,
            response_raw_path: None,
        };

        let row = invocation_archive_candidate_to_hourly_source_record(&candidate);

        assert_eq!(row.first_token_ms, Some(321.0));
    }
}

#[derive(Debug, FromRow, Clone)]
pub(crate) struct InvocationRawCompressionFieldCandidate {
    pub(crate) id: i64,
    pub(crate) occurred_at: String,
    pub(crate) raw_path: String,
}

#[derive(Debug, FromRow)]
struct RawPathReferenceCandidate {
    reference_kind: String,
    id: i64,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct ArchiveBatchFileRow {
    pub(crate) id: i64,
    pub(crate) file_path: String,
    pub(crate) coverage_start_at: Option<String>,
    pub(crate) coverage_end_at: Option<String>,
}

#[derive(Debug, FromRow)]
pub(crate) struct InvocationBucketPresenceRow {
    pub(crate) occurred_at: String,
    pub(crate) source: String,
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
pub(crate) struct TempSqliteCleanup(pub PathBuf);

pub(crate) fn temp_sqlite_source_meta_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.source-meta", path.display()))
}

pub(crate) fn remove_temp_sqlite_artifacts(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(temp_sqlite_source_meta_path(path));
}

impl Drop for TempSqliteCleanup {
    fn drop(&mut self) {
        remove_temp_sqlite_artifacts(&self.0);
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
    pub(crate) refreshed_batches: usize,
    pub(crate) account_rows_written: usize,
    pub(crate) missing_files: usize,
}

pub(crate) struct CountingWriter<W> {
    pub(crate) inner: W,
    pub(crate) bytes_written: u64,
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
pub(crate) struct TimestampedArchiveCandidate {
    pub(crate) id: i64,
    pub(crate) timestamp_value: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct DryRunBatchCount {
    pub(crate) month_key: String,
    pub(crate) row_count: i64,
}

pub(crate) const CODEX_INVOCATIONS_ARCHIVE_COLUMNS: &str = "id, invoke_id, occurred_at, source, model, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens, cost, cost_input, cost_cache_write, cost_cache_read, cost_output, cost_reasoning, status, error_message, failure_kind, failure_class, is_actionable, payload, raw_response, cost_estimated, price_version, request_raw_path, request_raw_codec, request_raw_size, request_raw_truncated, request_raw_truncated_reason, response_raw_path, response_raw_codec, response_raw_size, response_raw_truncated, response_raw_truncated_reason, detail_level, detail_pruned_at, detail_prune_reason, t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, first_token_ms, t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, created_at";
pub(crate) const FORWARD_PROXY_ATTEMPTS_ARCHIVE_COLUMNS: &str =
    "id, proxy_key, occurred_at, is_success, latency_ms, failure_kind, is_probe";
pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_COLUMNS: &str = "id, attempt_public_id, invoke_id, occurred_at, endpoint, route_mode, sticky_key, routing_source, routing_selection_audit_json, upstream_base_url_host, group_name_snapshot, proxy_binding_key_snapshot, request_model, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, requester_ip, started_at, finished_at, status, phase, http_status, downstream_http_status, failure_kind, error_message, downstream_error_message, connect_latency_ms, first_byte_latency_ms, stream_latency_ms, upstream_request_id, upstream_request_compression_algorithm, upstream_request_compression_mode, upstream_request_logical_body_bytes, upstream_request_transmitted_body_bytes, upstream_request_header_bytes_approx, upstream_response_body_bytes, upstream_response_header_bytes_approx, compact_support_status, compact_support_reason, request_summary_json, response_summary_json, response_raw_path, response_raw_codec, response_raw_size, response_raw_truncated, response_raw_truncated_reason, response_content_encoding, created_at";
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

pub(crate) fn spawn_data_retention_maintenance(
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
        loop {
            if run_data_retention_maintenance_best_effort(&state, &cancel, "startup").await {
                break;
            }
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("data retention maintenance received shutdown");
                    return;
                }
                _ = sleep(Duration::from_secs(BACKGROUND_DB_PRESSURE_RETRY_INTERVAL_SECS)) => {}
            }
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
                    run_data_retention_maintenance_best_effort(
                        &state,
                        &cancel,
                        "interval",
                    ).await;
                }
            }
        }
    })
}

pub(crate) async fn run_data_retention_maintenance_best_effort(
    state: &Arc<AppState>,
    cancel: &CancellationToken,
    trigger: &'static str,
) -> bool {
    match run_data_retention_maintenance(&state.pool, &state.config, None, Some(cancel)).await {
        Ok(summary) => {
            let touched_anything = summary.touched_anything();
            if touched_anything {
                let task_run = begin_system_task_run(
                    &state.pool,
                    SystemTaskKind::RetentionArchive,
                    trigger,
                    Some("retention maintenance completed a write pass".to_string()),
                )
                .await
                .ok();
                if let Some(handle) = task_run.as_ref() {
                    let (brief, detail) = summarize_retention_run_for_system_task(&summary);
                    finish_system_task_run_batched(
                        state.as_ref(),
                        handle,
                        SystemTaskStatus::Success,
                        Some(brief),
                        Some(detail),
                    )
                    .await;
                }
            }
            // A cold-compression rename changes the physical path as well. Reset the
            // incremental inventory so it cannot count both the retired and new blob.
            if (summary.raw_files_compressed > 0
                || summary.raw_files_removed > 0
                || summary.orphan_raw_files_removed > 0)
                && let Err(error) =
                    reset_retention_raw_payload_metrics_inventory(state.as_ref()).await
            {
                warn!(error = %error, "failed to reset system raw metrics inventory after retention");
            }
            invalidate_system_status_cache(state.as_ref()).await;
            touched_anything
        }
        Err(err) => {
            let pressure_error = crate::db_pressure::global_db_pressure_gate()
                .record_error("data_retention_maintenance", &err);
            retention_record_error("data_retention_maintenance", &err);
            let task_run = begin_system_task_run(
                &state.pool,
                SystemTaskKind::RetentionArchive,
                trigger,
                Some("retention maintenance failed".to_string()),
            )
            .await
            .ok();
            if let Some(handle) = task_run.as_ref() {
                finish_system_task_run_batched(
                    state.as_ref(),
                    handle,
                    SystemTaskStatus::Failed,
                    Some("retention maintenance failed".to_string()),
                    Some(err.to_string()),
                )
                .await;
            }
            warn!(trigger, error = %err, retry_soon = pressure_error, "failed to run retention maintenance");
            return !pressure_error;
        }
    };

    // Hourly rollups run through their own P2 scheduler. Retention used to invoke a
    // full refresh here after every committed batch, creating an uncoordinated long
    // write immediately after the maintenance micro-transaction released its permit.
    // Archive materialization already wakes the targeted repair path above.
    true
}

pub(crate) fn should_stop_data_retention_maintenance(shutdown: Option<&CancellationToken>) -> bool {
    let should_stop = shutdown.is_some_and(CancellationToken::is_cancelled);
    if should_stop {
        info!(
            "data retention maintenance stopped at a safe boundary because shutdown is in progress"
        );
    }
    should_stop
}

pub(crate) async fn run_data_retention_maintenance(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run_override: Option<bool>,
    shutdown: Option<&CancellationToken>,
) -> Result<RetentionRunSummary> {
    if let Some(shutdown) = shutdown {
        return RETENTION_SHUTDOWN
            .scope(
                shutdown.clone(),
                run_data_retention_maintenance_inner(
                    pool,
                    config,
                    dry_run_override,
                    Some(shutdown),
                ),
            )
            .await;
    }
    run_data_retention_maintenance_inner(pool, config, dry_run_override, None).await
}

async fn run_data_retention_maintenance_inner(
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
        // Hourly rollups are a separately scheduled P2 projection. Retention only checks its
        // coverage gate below; rebuilding it here used to add an unbounded write phase before
        // every archive pass.
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

    if dry_run {
        summary.model_route_rows_pruned =
            crate::upstream_accounts::count_expired_model_routes(pool).await? as usize;
    } else {
        let Some(admission) = acquire_retention_write_admission("model_route_purge").await else {
            return Ok(summary);
        };
        let execute_started = Instant::now();
        let candidate_limit = retention_candidate_limit(config, "model_route_purge");
        summary.model_route_rows_pruned =
            crate::upstream_accounts::purge_model_routes_bounded(pool, candidate_limit).await?
                as usize;
        retention_record_commit!(
            "model_route_purge",
            admission.admission_mode(),
            summary.model_route_rows_pruned,
            summary.model_route_rows_pruned.saturating_mul(128),
            Duration::ZERO,
            admission.lock_wait(),
            execute_started.elapsed(),
            Duration::ZERO,
            admission.p1_waiter_count,
            usize::from(summary.model_route_rows_pruned >= candidate_limit),
        );
        drop(admission);
    }

    let raw_compression =
        compress_cold_proxy_raw_payloads(pool, config, raw_path_fallback_root, dry_run)
            .await
            .context("failed to compress cold proxy raw payloads during retention")?;
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

    let payload_loss_days = config
        .invocation_success_full_days
        .min(config.invocation_max_days);
    let invocation_payload_retention_ready = dry_run
        || parallel_work_minute_coverage_ready_for_payload_retention(
            pool,
            shanghai_retention_cutoff(payload_loss_days).timestamp(),
        )
        .await
        .context("failed to verify parallel-work minute coverage before invocation retention")?;
    let pruned = if invocation_payload_retention_ready {
        prune_old_invocation_details(pool, config, raw_path_fallback_root, dry_run)
            .await
            .context("failed to prune old invocation details during retention")?
    } else {
        info!(
            payload_loss_days,
            "invocation detail pruning deferred until parallel-work minute coverage catches up"
        );
        (0, 0, 0)
    };
    summary.invocation_details_pruned += pruned.0;
    summary.archive_batches_touched += pruned.1;
    summary.raw_files_removed += pruned.2;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    let invocation_archive = if invocation_payload_retention_ready {
        archive_old_invocations(pool, config, raw_path_fallback_root, dry_run)
            .await
            .context("failed to archive old invocations during retention")?
    } else {
        info!(
            payload_loss_days,
            "invocation archival deferred until parallel-work minute coverage catches up"
        );
        (0, 0, 0)
    };
    summary.invocation_rows_archived += invocation_archive.0;
    summary.archive_batches_touched += invocation_archive.1;
    summary.raw_files_removed += invocation_archive.2;
    if !dry_run && (pruned.1 > 0 || invocation_archive.1 > 0) {
        let manifest_refresh = refresh_archive_upstream_activity_manifest(pool, config, false)
            .await
            .context("failed to refresh upstream activity manifest after invocation archive materialization")?;
        debug!(
            refreshed_batches = manifest_refresh.refreshed_batches,
            pending_batches = manifest_refresh.pending_batches,
            account_rows_written = manifest_refresh.account_rows_written,
            "refreshed upstream activity manifest before waking archive backfill"
        );
        wake_retention_startup_backfill_tasks(
            pool,
            &[
                StartupBackfillTask::UpstreamActivityArchives,
                StartupBackfillTask::HistoricalRollups,
            ],
            if pruned.1 > 0 {
                "invocation_detail_prune_archive_materialized"
            } else {
                "invocation_archive_materialized"
            },
        )
        .await?;
    }

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
    .await
    .context("failed to archive forward proxy attempts during retention")?;
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
    .await
    .context("failed to archive pool upstream request attempts during retention")?;
    summary.pool_upstream_request_attempt_rows_archived += pool_attempt_archive.0;
    summary.archive_batches_touched += pool_attempt_archive.1;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    let quota_archive = compact_old_quota_snapshots(pool, config, dry_run)
        .await
        .context("failed to compact old quota snapshots during retention")?;
    summary.quota_snapshot_rows_archived += quota_archive.0;
    summary.archive_batches_touched += quota_archive.1;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    summary.orphan_raw_files_removed +=
        sweep_orphan_proxy_raw_files(pool, config, raw_path_fallback_root, dry_run)
            .await
            .context("failed to sweep orphan proxy raw files during retention")?;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    let archive_ttl_cleanup = cleanup_expired_archive_batches(pool, config, dry_run)
        .await
        .context("failed to clean up expired archive batches during retention")?;
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

pub(crate) async fn run_best_effort_retention_pragma(
    pool: &Pool<Sqlite>,
    sql: &str,
    description: &'static str,
) -> Result<()> {
    let Some(admission) = acquire_retention_write_admission("retention_pragma").await else {
        return Ok(());
    };
    let execute_started = Instant::now();
    match sqlx::query(sql)
        .execute(pool)
        .await
        .with_context(|| format!("failed to run {description}"))
    {
        Ok(_) => {
            retention_record_commit!(
                "retention_pragma",
                admission.admission_mode(),
                1,
                0,
                Duration::ZERO,
                admission.lock_wait(),
                execute_started.elapsed(),
                Duration::ZERO,
                admission.p1_waiter_count,
                0,
            );
            Ok(())
        }
        Err(err) if is_sqlite_lock_error(&err) => {
            retention_record_error("retention_pragma", &err);
            warn!(error = %err, sql, "{description} skipped because the database is busy");
            Ok(())
        }
        Err(err) => {
            retention_record_error("retention_pragma", &err);
            Err(err)
        }
    }
}

pub(crate) async fn compress_cold_proxy_raw_payloads(
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

pub(crate) async fn compress_cold_proxy_raw_payloads_with_budget(
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
        retention_candidate_limit(config, "raw_compression")
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

        let (attempt_summary, attempt_hit_batch_limit) =
            compress_cold_pool_attempt_response_raw_lane(
                pool,
                config,
                raw_path_fallback_root,
                dry_run,
                batch_limit,
            )
            .await?;
        accumulate_raw_compression_summary(&mut summary, attempt_summary);

        if !request_hit_batch_limit && !response_hit_batch_limit && !attempt_hit_batch_limit {
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

async fn compress_cold_pool_attempt_response_raw_lane(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
    batch_limit: usize,
) -> Result<(RawCompressionPassSummary, bool)> {
    let cutoff = shanghai_local_cutoff_for_age_secs_string(config.proxy_raw_hot_secs);
    let archive_cutoff =
        shanghai_local_cutoff_string(config.pool_upstream_request_attempts_retention_days);
    let mut summary = RawCompressionPassSummary::default();
    let mut rows_processed = 0usize;
    let mut last_seen_occurred_at: Option<String> = None;
    let mut last_seen_id = 0_i64;

    while rows_processed < batch_limit {
        let candidates = sqlx::query_as::<_, InvocationRawCompressionFieldCandidate>(
            r#"
            SELECT id, occurred_at, response_raw_path AS raw_path
            FROM pool_upstream_request_attempts
            WHERE occurred_at < ?1
              AND occurred_at >= ?2
              AND response_raw_path IS NOT NULL
              AND response_raw_codec = ?3
              AND (?4 IS NULL OR occurred_at > ?4 OR (occurred_at = ?4 AND id > ?5))
            ORDER BY occurred_at ASC, id ASC
            LIMIT ?6
            "#,
        )
        .bind(&cutoff)
        .bind(&archive_cutoff)
        .bind(RAW_CODEC_IDENTITY)
        .bind(last_seen_occurred_at.as_deref())
        .bind(last_seen_id)
        .bind((batch_limit - rows_processed).max(1) as i64)
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        for candidate in candidates {
            last_seen_occurred_at = Some(candidate.occurred_at.clone());
            last_seen_id = candidate.id;
            rows_processed += 1;
            let outcome = match maybe_compress_proxy_raw_path(
                pool,
                candidate.id,
                "attempt_response",
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
                        field = "attempt_response",
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
                let references_updated = replace_proxy_raw_path_references(
                    pool,
                    config,
                    &candidate.raw_path,
                    &next_path,
                    &next_codec,
                )
                .await?;
                if let Some(path) = outcome.old_exact_path.as_deref()
                    && references_updated
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
            if rows_processed >= batch_limit {
                break;
            }
        }
    }
    Ok((summary, rows_processed >= batch_limit))
}

pub(crate) fn accumulate_raw_compression_summary(
    target: &mut RawCompressionPassSummary,
    next: RawCompressionPassSummary,
) {
    target.files_considered += next.files_considered;
    target.files_compressed += next.files_compressed;
    target.bytes_before += next.bytes_before;
    target.bytes_after += next.bytes_after;
    target.estimated_bytes_after += next.estimated_bytes_after;
}

pub(crate) async fn compress_cold_proxy_raw_payload_lane(
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
    let mut rows_processed = 0usize;
    let mut last_seen_occurred_at: Option<String> = None;
    let mut last_seen_id = 0_i64;

    while rows_processed < batch_limit {
        let remaining = (batch_limit - rows_processed) as i64;
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
            rows_processed += 1;

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
                let references_updated = replace_proxy_raw_path_references(
                    pool,
                    config,
                    &candidate.raw_path,
                    &next_path,
                    &next_codec,
                )
                .await?;

                if let Some(path) = outcome.old_exact_path.as_deref()
                    && references_updated
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

            if rows_processed >= batch_limit {
                break;
            }
        }
    }

    let hit_batch_limit = rows_processed >= batch_limit;
    Ok((summary, hit_batch_limit))
}

pub(crate) async fn maybe_compress_proxy_raw_path(
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
    if target_path.exists() {
        return Ok(RawCompressionFileOutcome {
            candidate_counted: true,
            bytes_before,
            new_db_path: Some(target_db_path),
            new_codec: Some(RAW_CODEC_GZIP.to_string()),
            old_exact_path: Some(source_path),
            ..RawCompressionFileOutcome::default()
        });
    }
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

pub(crate) fn compress_file_to_gzip(source: &Path, destination: &Path) -> Result<u64> {
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

pub(crate) fn estimate_gzip_file_size(source: &Path) -> Result<u64> {
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

pub(crate) fn raw_payload_compressed_db_path(raw_path: &str) -> String {
    if raw_path.ends_with(".gz") {
        raw_path.to_string()
    } else {
        format!("{raw_path}.gz")
    }
}

pub(crate) fn raw_codec_from_path(raw_path: Option<&str>) -> String {
    match raw_path {
        Some(path) if path.ends_with(".gz") => RAW_CODEC_GZIP.to_string(),
        _ => RAW_CODEC_IDENTITY.to_string(),
    }
}

pub(crate) fn raw_codec_is_identity(raw_codec: Option<&str>) -> bool {
    matches!(raw_codec, Some(RAW_CODEC_IDENTITY) | None)
}

pub(crate) fn raw_payload_compressed_file_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.gz", path.display()))
}

pub(crate) async fn replace_proxy_raw_path_references(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    old_path: &str,
    next_path: &str,
    next_codec: &str,
) -> Result<bool> {
    loop {
        let candidate_limit = retention_candidate_limit(config, "raw_path_reference_update")
            .min(RETENTION_WRITE_MAX_ROWS);
        let mut candidates = sqlx::query_as::<_, RawPathReferenceCandidate>(
            r#"
            SELECT reference_kind, id
            FROM (
                SELECT 'invocation_request' AS reference_kind, id
                FROM codex_invocations
                WHERE request_raw_path = ?1
                UNION ALL
                SELECT 'invocation_response' AS reference_kind, id
                FROM codex_invocations
                WHERE response_raw_path = ?1
                UNION ALL
                SELECT 'attempt_response' AS reference_kind, id
                FROM pool_upstream_request_attempts
                WHERE response_raw_path = ?1
            )
            ORDER BY reference_kind ASC, id ASC
            LIMIT ?2
            "#,
        )
        .bind(old_path)
        .bind(candidate_limit.saturating_add(1) as i64)
        .fetch_all(pool)
        .await?;
        if candidates.is_empty() {
            break;
        }

        let candidate_remaining_hint = usize::from(candidates.len() > candidate_limit);
        candidates.truncate(candidate_limit);
        let mut by_kind = BTreeMap::<String, Vec<i64>>::new();
        for candidate in &candidates {
            by_kind
                .entry(candidate.reference_kind.clone())
                .or_default()
                .push(candidate.id);
        }
        let Some(admission) = acquire_retention_write_admission("raw_path_reference_update").await
        else {
            return Ok(false);
        };
        let execute_started = Instant::now();
        let mut tx = pool.begin().await?;
        let mut updated = 0usize;
        for (reference_kind, ids) in by_kind {
            let (table, path_column, codec_column) = match reference_kind.as_str() {
                "invocation_request" => {
                    ("codex_invocations", "request_raw_path", "request_raw_codec")
                }
                "invocation_response" => (
                    "codex_invocations",
                    "response_raw_path",
                    "response_raw_codec",
                ),
                "attempt_response" => (
                    "pool_upstream_request_attempts",
                    "response_raw_path",
                    "response_raw_codec",
                ),
                _ => continue,
            };
            let mut update =
                QueryBuilder::<Sqlite>::new(format!("UPDATE {table} SET {path_column} = "));
            update
                .push_bind(next_path)
                .push(format!(", {codec_column} = "))
                .push_bind(next_codec)
                .push(format!(" WHERE {path_column} = "))
                .push_bind(old_path)
                .push(" AND id IN (");
            {
                let mut separated = update.separated(", ");
                for id in &ids {
                    separated.push_bind(id);
                }
            }
            update.push(")");
            updated += update.build().execute(tx.as_mut()).await?.rows_affected() as usize;
        }
        let commit_started = Instant::now();
        tx.commit().await?;
        retention_record_commit!(
            "raw_path_reference_update",
            admission.admission_mode(),
            updated,
            updated.saturating_mul(192),
            Duration::ZERO,
            admission.lock_wait(),
            commit_started.duration_since(execute_started),
            commit_started.elapsed(),
            admission.p1_waiter_count,
            candidate_remaining_hint,
        );
        drop(admission);
    }
    debug!(
        old_path,
        next_path, next_codec, "propagated shared proxy raw path replacement"
    );
    Ok(true)
}

async fn wake_retention_startup_backfill_tasks(
    pool: &Pool<Sqlite>,
    tasks: &[StartupBackfillTask],
    wake_reason: &'static str,
) -> Result<u64> {
    let Some(admission) = acquire_retention_write_admission("archive_backfill_wake").await else {
        return Ok(0);
    };
    let execute_started = Instant::now();
    let woken = wake_startup_backfill_tasks(pool, tasks, wake_reason).await?;
    retention_record_commit!(
        "archive_backfill_wake",
        admission.admission_mode(),
        woken as usize,
        tasks.len().saturating_mul(256),
        Duration::ZERO,
        admission.lock_wait(),
        execute_started.elapsed(),
        Duration::ZERO,
        admission.p1_waiter_count,
        0,
    );
    Ok(woken)
}

async fn reset_retention_raw_payload_metrics_inventory(state: &AppState) -> Result<()> {
    loop {
        let Some(admission) =
            acquire_retention_write_admission("raw_metrics_inventory_reset").await
        else {
            return Ok(());
        };
        let candidate_limit =
            retention_candidate_limit(&state.config, "raw_metrics_inventory_reset");
        let execute_started = Instant::now();
        let outcome =
            reset_system_raw_payload_metrics_inventory_batch(state, candidate_limit).await?;
        retention_record_commit!(
            "raw_metrics_inventory_reset",
            admission.admission_mode(),
            outcome.removed_path_count.saturating_add(1),
            outcome
                .removed_path_count
                .saturating_mul(128)
                .saturating_add(256),
            Duration::ZERO,
            admission.lock_wait(),
            execute_started.elapsed(),
            Duration::ZERO,
            admission.p1_waiter_count,
            usize::from(!outcome.complete),
        );
        if outcome.complete {
            return Ok(());
        }
    }
}

pub(crate) fn locate_existing_proxy_raw_path(
    path: &str,
    fallback_root: Option<&Path>,
) -> Option<PathBuf> {
    resolved_raw_path_candidates(path, fallback_root)
        .into_iter()
        .find(|candidate| candidate.exists())
}

pub(crate) fn locate_existing_proxy_raw_compressed_path(
    path: &str,
    fallback_root: Option<&Path>,
) -> Option<PathBuf> {
    resolved_raw_path_candidates(&raw_payload_compressed_db_path(path), fallback_root)
        .into_iter()
        .find(|candidate| candidate.exists())
}

pub(crate) fn delete_exact_proxy_raw_path(
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

async fn filter_unreferenced_proxy_raw_paths(
    pool: &Pool<Sqlite>,
    raw_paths: &[Option<String>],
) -> Result<Vec<Option<String>>> {
    let candidates = raw_paths
        .iter()
        .flatten()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let mut unreferenced = Vec::with_capacity(candidates.len());
    for path in candidates {
        let referenced = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT EXISTS(
              SELECT 1 FROM proxy_raw_payload_blob_links
              WHERE raw_path = ?1
              UNION ALL
              SELECT 1 FROM codex_invocations
              WHERE request_raw_path = ?1 OR response_raw_path = ?1
              UNION ALL
              SELECT 1 FROM pool_upstream_request_attempts
              WHERE response_raw_path = ?1
            )
            "#,
        )
        .bind(&path)
        .fetch_one(pool)
        .await?;
        if referenced == 0 {
            unreferenced.push(Some(path));
        }
    }
    Ok(unreferenced)
}

pub(crate) async fn prune_old_invocation_details(
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
            SELECT id, occurred_at, request_raw_path, response_raw_path,
                   COALESCE(length(payload), 0) + COALESCE(length(raw_response), 0) + 512
                       AS estimated_write_bytes
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
            SELECT id, occurred_at, request_raw_path, response_raw_path,
                   COALESCE(length(payload), 0) + COALESCE(length(raw_response), 0) + 512
                       AS estimated_write_bytes
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
        let candidate_limit = retention_candidate_limit(config, "invocation_detail_prune");
        let candidates = sqlx::query_as::<_, InvocationDetailPruneCandidate>(&sql)
            .bind(DETAIL_LEVEL_FULL)
            .bind(&prune_cutoff)
            .bind(&archive_cutoff)
            .bind(candidate_limit as i64)
            .fetch_all(pool)
            .await?;

        if candidates.is_empty() {
            break;
        }

        let candidate_remaining_hint = usize::from(candidates.len() >= candidate_limit);
        let mut by_group: BTreeMap<String, Vec<InvocationDetailPruneCandidate>> = BTreeMap::new();
        for candidate in candidates {
            let group_key = invocation_archive_group_key(config, &candidate.occurred_at)?;
            by_group.entry(group_key).or_default().push(candidate);
        }

        for (group_key, group) in by_group {
            let group = take_retention_micro_batch(group, |candidate| {
                candidate.estimated_write_bytes.max(1) as usize
            });
            let prepare_started = Instant::now();
            let ids = group
                .iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            let raw_paths = group
                .iter()
                .flat_map(|candidate| {
                    [
                        candidate.request_raw_path.clone(),
                        candidate.response_raw_path.clone(),
                    ]
                })
                .collect::<Vec<_>>();
            let Some(mut archive_outcome) = retention_prepared_batch_or_deferred(
                match archive_layout_for_dataset(config, spec.dataset) {
                    ArchiveBatchLayout::LegacyMonth => {
                        archive_rows_into_month_batch(pool, config, spec, &group_key, &ids).await
                    }
                    ArchiveBatchLayout::SegmentV1 => {
                        archive_rows_into_segment_batch(pool, config, spec, &group_key, &ids).await
                    }
                },
            )?
            else {
                return Ok((rows_pruned, archive_batches, raw_files_removed));
            };
            set_archive_batch_coverage_from_local_rows(
                &mut archive_outcome,
                group.iter().map(|candidate| candidate.occurred_at.as_str()),
                Some(config.invocation_archive_ttl_days),
            )?;
            let pruned_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
            let prepare_elapsed = prepare_started.elapsed();
            let Some(admission) =
                acquire_retention_write_admission("invocation_detail_prune").await
            else {
                return Ok((rows_pruned, archive_batches, raw_files_removed));
            };
            let execute_started = Instant::now();
            let mut tx = pool.begin().await?;
            upsert_archive_batch_manifest(tx.as_mut(), &archive_outcome).await?;
            mark_archive_batch_historical_rollups_materialized_tx(
                tx.as_mut(),
                spec.dataset,
                &archive_outcome.file_path,
            )
            .await?;
            let mut query = QueryBuilder::<Sqlite>::new(
                "UPDATE codex_invocations SET payload = CASE WHEN json_valid(payload) AND (json_extract(payload, '$.upstreamAccountId') IS NOT NULL OR json_extract(payload, '$.requestModel') IS NOT NULL OR json_extract(payload, '$.responseModel') IS NOT NULL OR json_extract(payload, '$.reasoningEffort') IS NOT NULL OR json_extract(payload, '$.requestCompressionAlgorithm') IS NOT NULL) THEN json_patch(json_patch(json_patch(json_patch(json_patch('{}', CASE WHEN json_extract(payload, '$.upstreamAccountId') IS NOT NULL THEN json_object('upstreamAccountId', json_extract(payload, '$.upstreamAccountId')) ELSE '{}' END), CASE WHEN json_extract(payload, '$.requestModel') IS NOT NULL THEN json_object('requestModel', json_extract(payload, '$.requestModel')) ELSE '{}' END), CASE WHEN json_extract(payload, '$.responseModel') IS NOT NULL THEN json_object('responseModel', json_extract(payload, '$.responseModel')) ELSE '{}' END), CASE WHEN json_extract(payload, '$.reasoningEffort') IS NOT NULL THEN json_object('reasoningEffort', json_extract(payload, '$.reasoningEffort')) ELSE '{}' END), CASE WHEN json_extract(payload, '$.requestCompressionAlgorithm') IS NOT NULL THEN json_object('requestCompressionAlgorithm', json_extract(payload, '$.requestCompressionAlgorithm')) ELSE '{}' END) ELSE NULL END, raw_response = '', request_raw_path = NULL, request_raw_codec = 'identity', request_raw_size = NULL, request_raw_truncated = 0, request_raw_truncated_reason = NULL, response_raw_path = NULL, response_raw_codec = 'identity', response_raw_size = NULL, response_raw_truncated = 0, response_raw_truncated_reason = NULL, detail_level = ",
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
            if let Some(latest) = group
                .iter()
                .map(|candidate| candidate.occurred_at.as_str())
                .max()
            {
                record_parallel_work_unrecoverable_detail_tx(tx.as_mut(), latest).await?;
            }
            let commit_started = Instant::now();
            tx.commit().await?;
            retention_record_commit!(
                "invocation_detail_prune",
                admission.admission_mode(),
                group.len(),
                group
                    .iter()
                    .map(|candidate| candidate.estimated_write_bytes.max(1) as usize)
                    .sum(),
                prepare_elapsed,
                admission.lock_wait(),
                commit_started.duration_since(execute_started),
                commit_started.elapsed(),
                admission.p1_waiter_count,
                candidate_remaining_hint,
            );
            drop(admission);
            rows_pruned += group.len();
            archive_batches += 1;

            let raw_paths = filter_unreferenced_proxy_raw_paths(pool, &raw_paths).await?;
            raw_files_removed += delete_proxy_raw_paths(&raw_paths, raw_path_fallback_root)?;
        }
    }

    Ok((rows_pruned, archive_batches, raw_files_removed))
}

pub(crate) async fn archive_old_invocations(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
) -> Result<(usize, usize, usize)> {
    let cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let spec = archive_table_spec("codex_invocations");
    let candidate_limit = retention_candidate_limit(config, "invocation_archive");

    if dry_run {
        let candidates = sqlx::query_as::<_, InvocationArchiveCandidate>(
            r#"
            SELECT
                id,
                occurred_at,
                source,
                status,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                total_tokens,
                cost,
                first_token_ms,
                payload,
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
                input_tokens,
                output_tokens,
                cache_input_tokens,
                total_tokens,
                cost,
                first_token_ms,
                payload,
                request_raw_path,
                response_raw_path
            FROM codex_invocations
            WHERE occurred_at < ?1
            ORDER BY occurred_at ASC, id ASC
            LIMIT ?2
            "#,
        )
        .bind(&cutoff)
        .bind(candidate_limit as i64)
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        let candidate_remaining_hint = usize::from(candidates.len() >= candidate_limit);
        let mut by_group: BTreeMap<String, Vec<InvocationArchiveCandidate>> = BTreeMap::new();
        for candidate in candidates {
            let group_key = invocation_archive_group_key(config, &candidate.occurred_at)?;
            by_group.entry(group_key).or_default().push(candidate);
        }

        for (group_key, group) in by_group {
            let group = take_retention_micro_batch(group, |candidate| {
                candidate.payload.as_deref().map_or(256, str::len).max(1)
            });
            let prepare_started = Instant::now();
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
                .map(invocation_archive_candidate_to_hourly_source_record)
                .collect::<Vec<_>>();
            let Some(mut archive_outcome) = retention_prepared_batch_or_deferred(
                match archive_layout_for_dataset(config, spec.dataset) {
                    ArchiveBatchLayout::LegacyMonth => {
                        archive_rows_into_month_batch(pool, config, spec, &group_key, &ids).await
                    }
                    ArchiveBatchLayout::SegmentV1 => {
                        archive_rows_into_segment_batch(pool, config, spec, &group_key, &ids).await
                    }
                },
            )?
            else {
                return Ok((rows_archived, archive_batches, raw_files_removed));
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
            let prepare_elapsed = prepare_started.elapsed();
            let Some(admission) = acquire_retention_write_admission("invocation_archive").await
            else {
                return Ok((rows_archived, archive_batches, raw_files_removed));
            };
            let execute_started = Instant::now();
            let mut tx = pool.begin().await?;
            // P2 normally advances this cursor before retention. Rows beyond it would be
            // deleted before the regular replay can observe them, so materialize just those
            // rows in this same archive transaction before claiming the archive is covered.
            let live_rollup_cursor =
                load_hourly_rollup_live_progress_tx(tx.as_mut(), HOURLY_ROLLUP_DATASET_INVOCATIONS)
                    .await?;
            let unprojected_rows = materialized_rows
                .iter()
                .filter(|row| row.id > live_rollup_cursor)
                .cloned()
                .collect::<Vec<_>>();
            if !unprojected_rows.is_empty() {
                upsert_invocation_hourly_rollups_tx(
                    tx.as_mut(),
                    &unprojected_rows,
                    &INVOCATION_HOURLY_ROLLUP_TARGETS,
                )
                .await?;

                // The all-time reader can safely read a raw tail only after a
                // contiguous live prefix. Do not leap over newer retained rows
                // that happened to receive lower IDs than this archive batch.
                let prefix_end = unprojected_rows
                    .iter()
                    .map(|row| row.id)
                    .max()
                    .expect("unprojected rows are non-empty");
                let prefix_row_count = sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM codex_invocations WHERE id > ?1 AND id <= ?2",
                )
                .bind(live_rollup_cursor)
                .bind(prefix_end)
                .fetch_one(tx.as_mut())
                .await?;
                if prefix_row_count == unprojected_rows.len() as i64 {
                    save_hourly_rollup_live_progress_tx(
                        tx.as_mut(),
                        HOURLY_ROLLUP_DATASET_INVOCATIONS,
                        prefix_end,
                    )
                    .await?;
                }
            }
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
            let commit_started = Instant::now();
            tx.commit().await?;
            retention_record_commit!(
                "invocation_archive",
                admission.admission_mode(),
                group.len(),
                group
                    .iter()
                    .map(|candidate| {
                        candidate
                            .payload
                            .as_deref()
                            .map_or(256, |payload| payload.len())
                    })
                    .sum(),
                prepare_elapsed,
                admission.lock_wait(),
                commit_started.duration_since(execute_started),
                commit_started.elapsed(),
                admission.p1_waiter_count,
                candidate_remaining_hint,
            );
            drop(admission);
            rows_archived += group.len();
            archive_batches += 1;
            let raw_paths = filter_unreferenced_proxy_raw_paths(pool, &raw_paths).await?;
            raw_files_removed += delete_proxy_raw_paths(&raw_paths, raw_path_fallback_root)?;
        }
    }

    Ok((rows_archived, archive_batches, raw_files_removed))
}

pub(crate) async fn archive_timestamped_dataset(
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
        let candidate_limit = retention_candidate_limit(config, "timestamped_archive");
        let candidates = sqlx::query_as::<_, TimestampedArchiveCandidate>(select_sql)
            .bind(&cutoff)
            .bind(candidate_limit as i64)
            .fetch_all(pool)
            .await?;

        if candidates.is_empty() {
            break;
        }

        let candidate_remaining_hint = usize::from(candidates.len() >= candidate_limit);
        let mut by_month: BTreeMap<String, Vec<TimestampedArchiveCandidate>> = BTreeMap::new();
        for candidate in candidates {
            let month_key =
                archive_timestamped_dataset_month_key(spec.dataset, &candidate.timestamp_value)?;
            by_month.entry(month_key).or_default().push(candidate);
        }

        for (month_key, group) in by_month {
            let group = take_retention_micro_batch(group, |_| 256);
            let prepare_started = Instant::now();
            let ids = group
                .iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            let pool_attempt_raw_paths = if spec.dataset == "pool_upstream_request_attempts" {
                let placeholders = std::iter::repeat_n("?", ids.len())
                    .collect::<Vec<_>>()
                    .join(",");
                let query = format!(
                    "SELECT response_raw_path FROM pool_upstream_request_attempts WHERE id IN ({placeholders})"
                );
                let mut query_builder = sqlx::query_scalar::<_, Option<String>>(&query);
                for id in &ids {
                    query_builder = query_builder.bind(id);
                }
                query_builder
                    .fetch_all(pool)
                    .await?
                    .into_iter()
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            let recreated_pool_upstream_month_archive = if spec.dataset
                == "pool_upstream_request_attempts"
            {
                let archive_file_path = archive_batch_file_path(config, spec.dataset, &month_key)?
                    .to_string_lossy()
                    .to_string();
                pool_upstream_month_archive_reappeared_after_cleanup(pool, &archive_file_path)
                    .await?
            } else {
                false
            };
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
            let Some(mut archive_outcome) = retention_prepared_batch_or_deferred(
                archive_rows_into_month_batch(pool, config, spec, &month_key, &ids).await,
            )?
            else {
                return Ok((rows_archived, archive_batches));
            };
            if spec.dataset == "pool_upstream_request_attempts" {
                set_archive_batch_coverage_from_local_rows(
                    &mut archive_outcome,
                    group
                        .iter()
                        .map(|candidate| candidate.timestamp_value.as_str()),
                    Some(config.pool_upstream_request_attempts_archive_ttl_days),
                )?;
                if recreated_pool_upstream_month_archive
                    && archive_outcome.row_count == ids.len() as i64
                {
                    archive_outcome.archive_expires_at =
                        Some(shanghai_archive_expiry_from_reference_timestamp(
                            &format_utc_iso(Utc::now()),
                            config.pool_upstream_request_attempts_archive_ttl_days,
                        )?);
                }
            } else {
                set_archive_batch_coverage_from_utc_rows(
                    &mut archive_outcome,
                    group
                        .iter()
                        .map(|candidate| candidate.timestamp_value.as_str()),
                )?;
            }
            let prepare_elapsed = prepare_started.elapsed();
            let Some(admission) = acquire_retention_write_admission("timestamped_archive").await
            else {
                return Ok((rows_archived, archive_batches));
            };
            let execute_started = Instant::now();
            let mut tx = pool.begin().await?;
            upsert_archive_batch_manifest(tx.as_mut(), &archive_outcome).await?;
            if spec.dataset == "pool_upstream_request_attempts" {
                let archive_batch_id = load_archive_batch_id_for_file_tx(
                    tx.as_mut(),
                    spec.dataset,
                    &archive_outcome.month_key,
                    &archive_outcome.file_path,
                )
                .await?;
                let archive_file_contains_only_new_rows =
                    archive_outcome.row_count == ids.len() as i64;
                let node_health_archive_already_replayed = hourly_rollup_archive_replayed_tx(
                    tx.as_mut(),
                    POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
                    spec.dataset,
                    &archive_outcome.file_path,
                )
                .await?;
                let node_health_hourly_archive_already_replayed =
                    hourly_rollup_archive_replayed_tx(
                        tx.as_mut(),
                        POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
                        spec.dataset,
                        &archive_outcome.file_path,
                    )
                    .await?;
                cache_pool_upstream_node_health_archive_rows_from_live_ids_tx(
                    tx.as_mut(),
                    &archive_outcome.file_path,
                    &ids,
                )
                .await?;
                refresh_pool_upstream_node_health_hourly_archive_rows_from_cache_tx(
                    tx.as_mut(),
                    archive_batch_id,
                    &archive_outcome.file_path,
                )
                .await?;
                if archive_file_contains_only_new_rows
                    || node_health_hourly_archive_already_replayed
                {
                    mark_hourly_rollup_archive_replayed_tx(
                        tx.as_mut(),
                        POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
                        spec.dataset,
                        &archive_outcome.file_path,
                    )
                    .await?;
                } else {
                    sqlx::query(
                        r#"
                        DELETE FROM hourly_rollup_archive_replay
                        WHERE target = ?1
                          AND dataset = ?2
                          AND file_path = ?3
                        "#,
                    )
                    .bind(POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET)
                    .bind(spec.dataset)
                    .bind(&archive_outcome.file_path)
                    .execute(tx.as_mut())
                    .await?;
                }
                if archive_file_contains_only_new_rows || node_health_archive_already_replayed {
                    mark_hourly_rollup_archive_replayed_tx(
                        tx.as_mut(),
                        POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
                        spec.dataset,
                        &archive_outcome.file_path,
                    )
                    .await?;
                    mark_archive_batch_historical_rollups_materialized_tx(
                        tx.as_mut(),
                        spec.dataset,
                        &archive_outcome.file_path,
                    )
                    .await?;
                } else {
                    sqlx::query(
                        r#"
                        DELETE FROM hourly_rollup_archive_replay
                        WHERE target = ?1
                          AND dataset = ?2
                          AND file_path = ?3
                        "#,
                    )
                    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
                    .bind(spec.dataset)
                    .bind(&archive_outcome.file_path)
                    .execute(tx.as_mut())
                    .await?;
                    sqlx::query(
                        r#"
                        UPDATE archive_batches
                        SET historical_rollups_materialized_at = NULL
                        WHERE dataset = ?1
                          AND file_path = ?2
                        "#,
                    )
                    .bind(spec.dataset)
                    .bind(&archive_outcome.file_path)
                    .execute(tx.as_mut())
                    .await?;
                }
            } else {
                mark_archive_batch_historical_rollups_materialized_tx(
                    tx.as_mut(),
                    spec.dataset,
                    &archive_outcome.file_path,
                )
                .await?;
            }
            delete_rows_by_ids(tx.as_mut(), spec.dataset, &ids).await?;
            mark_retention_archived_hourly_rollup_targets_tx(
                tx.as_mut(),
                spec.dataset,
                &[],
                &materialized_forward_proxy_rows,
            )
            .await?;
            let commit_started = Instant::now();
            tx.commit().await?;
            retention_record_commit!(
                "timestamped_archive",
                admission.admission_mode(),
                group.len(),
                group.len().saturating_mul(256),
                prepare_elapsed,
                admission.lock_wait(),
                commit_started.duration_since(execute_started),
                commit_started.elapsed(),
                admission.p1_waiter_count,
                candidate_remaining_hint,
            );
            drop(admission);
            rows_archived += group.len();
            archive_batches += 1;
            if spec.dataset == "pool_upstream_request_attempts" {
                let raw_paths =
                    filter_unreferenced_proxy_raw_paths(pool, &pool_attempt_raw_paths).await?;
                let _ = delete_proxy_raw_paths(&raw_paths, config.database_path.parent())?;
            }
        }
    }

    Ok((rows_archived, archive_batches))
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

#[cfg(test)]
mod retention_write_budget_tests {
    use super::*;

    #[test]
    fn retention_write_budget_adapts_without_exceeding_hard_bounds() {
        let mut budget = RetentionWriteBudget::default();
        assert_eq!(budget.candidate_limit(1_000), RETENTION_WRITE_INITIAL_ROWS);

        assert!(budget.observe_commit(4, 4 * 256, Duration::from_millis(251)));
        assert_eq!(budget.candidate_limit(1_000), 2);

        assert!(budget.observe_commit(1, RETENTION_WRITE_MAX_BYTES + 1, Duration::ZERO));
        assert_eq!(budget.candidate_limit(1_000), 1);

        for _ in 0..100 {
            assert!(!budget.observe_commit(1, 256, Duration::from_millis(1)));
        }
        assert!(budget.candidate_limit(1_000) <= RETENTION_WRITE_MAX_ROWS);
    }

    #[test]
    fn retention_micro_batch_keeps_a_single_oversized_row_losslessly() {
        let selected =
            take_retention_micro_batch(vec![2 * RETENTION_WRITE_MAX_BYTES, 128], |value| *value);
        assert_eq!(selected, vec![2 * RETENTION_WRITE_MAX_BYTES]);
    }
}

#[derive(Debug, FromRow)]
pub(crate) struct ArchiveExpiryBackfillCandidate {
    pub(crate) id: i64,
    pub(crate) coverage_end_at: String,
}
