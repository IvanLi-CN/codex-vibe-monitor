use super::*;

use sqlx::FromRow;
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const RETENTION_FAIRNESS_INTERVAL: Duration = Duration::from_secs(15);
const RETENTION_WRITE_TARGET: Duration = Duration::from_millis(200);
const RETENTION_WRITE_WARNING: Duration = Duration::from_millis(250);
const RETENTION_WRITE_INITIAL_ROWS: usize = 4;
pub(super) const RETENTION_WRITE_MAX_ROWS: usize = 64;
const RETENTION_WRITE_MAX_BYTES: usize = 1024 * 1024;
const SYSTEM_TASK_RUN_RETENTION_KEEP_RECENT: i64 = 200;
const SYSTEM_TASK_RUN_RETENTION_TERMINAL_BATCH_ROWS: usize = 500;
const SYSTEM_TASK_RUN_RETENTION_MAX_ROWS_PER_PASS: usize = 5_000;
const SYSTEM_TASK_RUN_RETENTION_PASS_INTERVAL: Duration = Duration::from_secs(15);
const SYSTEM_TASK_RUN_RETENTION_PRESSURE_BACKOFF: Duration = Duration::from_secs(5 * 60);

static SYSTEM_TASK_RUN_RETENTION_NEXT_PASS_EPOCH_MS: AtomicU64 = AtomicU64::new(0);
static SYSTEM_TASK_RUN_RETENTION_SCHEDULE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

tokio::task_local! {
    static RETENTION_SHUTDOWN: CancellationToken;
}

static RETENTION_DEFER_GENERATION: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

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
    retention_adaptive_candidate_limit(config.retention_batch_rows, operation)
}

fn retention_adaptive_candidate_limit(configured_limit: usize, operation: &'static str) -> usize {
    let mut health = RETENTION_WRITE_HEALTH
        .lock()
        .expect("retention write health");
    retention_adaptive_candidate_limit_from_state(&mut health, configured_limit, operation)
}

fn retention_adaptive_candidate_limit_from_state(
    health: &mut RetentionWriteHealthState,
    configured_limit: usize,
    operation: &'static str,
) -> usize {
    health
        .budgets
        .entry(operation)
        .or_default()
        .candidate_limit(configured_limit)
}

pub(super) fn retention_micro_batch_limit(config: &AppConfig, operation: &'static str) -> usize {
    retention_candidate_limit(config, operation).min(RETENTION_WRITE_MAX_ROWS)
}

fn retention_record_defer(operation: &'static str, reason: impl ToString) {
    RETENTION_DEFER_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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

pub(crate) fn retention_defer_generation() -> u64 {
    RETENTION_DEFER_GENERATION.load(std::sync::atomic::Ordering::Relaxed)
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
    let mut health = RETENTION_WRITE_HEALTH
        .lock()
        .expect("retention write health");
    let breached = observe_retention_write_commit(&mut health, &commit);
    drop(health);
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

fn observe_retention_write_commit(
    health: &mut RetentionWriteHealthState,
    commit: &RetentionWriteCommit,
) -> bool {
    let operation = commit.operation;
    let admission_mode = commit.admission_mode;
    let rows = commit.rows;
    let estimated_bytes = commit.estimated_bytes;
    let prepare_elapsed = commit.prepare_elapsed;
    let lock_wait = commit.lock_wait;
    let execute_elapsed = commit.execute_elapsed;
    let commit_elapsed = commit.commit_elapsed;
    let p1_waiter_count = commit.p1_waiter_count;
    let candidate_remaining_hint = commit.candidate_remaining_hint;
    let elapsed = execute_elapsed.saturating_add(commit_elapsed);
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
    breached
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
    pub(crate) deferred: bool,
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
    pub(crate) system_task_run_rows_pruned: usize,
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
            || self.system_task_run_rows_pruned > 0
    }
}

#[derive(Debug, FromRow)]
struct SystemTaskRunRetentionCandidate {
    id: i64,
}

fn system_task_run_retention_now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn system_task_run_retention_schedule_next(now_epoch_ms: u64, delay: Duration) {
    let next = now_epoch_ms.saturating_add(delay.as_millis().min(u64::MAX as u128) as u64);
    let mut current = SYSTEM_TASK_RUN_RETENTION_NEXT_PASS_EPOCH_MS.load(Ordering::Acquire);
    while next > current {
        match SYSTEM_TASK_RUN_RETENTION_NEXT_PASS_EPOCH_MS.compare_exchange(
            current,
            next,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return,
            Err(actual) => current = actual,
        }
    }
}

fn system_task_run_retention_pass_is_due(now_epoch_ms: u64) -> bool {
    let _schedule_guard = SYSTEM_TASK_RUN_RETENTION_SCHEDULE_LOCK
        .lock()
        .expect("system task retention schedule lock");
    let next = SYSTEM_TASK_RUN_RETENTION_NEXT_PASS_EPOCH_MS.load(Ordering::Acquire);
    if next > now_epoch_ms {
        return false;
    }
    SYSTEM_TASK_RUN_RETENTION_NEXT_PASS_EPOCH_MS
        .compare_exchange(
            next,
            now_epoch_ms.saturating_add(
                SYSTEM_TASK_RUN_RETENTION_PASS_INTERVAL
                    .as_millis()
                    .min(u64::MAX as u128) as u64,
            ),
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_ok()
}

#[cfg(test)]
pub(crate) fn reset_system_task_run_retention_schedule() {
    let _schedule_guard = SYSTEM_TASK_RUN_RETENTION_SCHEDULE_LOCK
        .lock()
        .expect("system task retention schedule lock");
    SYSTEM_TASK_RUN_RETENTION_NEXT_PASS_EPOCH_MS.store(0, Ordering::Release);
}

pub(crate) fn system_task_run_retention_handle_pressure(error: &anyhow::Error) -> bool {
    if !crate::db_pressure::is_db_pressure_error(error) {
        return false;
    }
    let now_epoch_ms = system_task_run_retention_now_epoch_ms();
    system_task_run_retention_schedule_next(
        now_epoch_ms,
        SYSTEM_TASK_RUN_RETENTION_PRESSURE_BACKOFF,
    );
    retention_record_defer("system_task_run_retention", "sqlite_pressure");
    crate::db_pressure::global_db_pressure_gate().record_error("system_task_run_retention", error);
    retention_record_error("system_task_run_retention", error);
    warn!(error = %error, "system task run retention deferred after SQLite pressure");
    true
}

fn system_task_run_retention_admission_requires_pressure_backoff(
    reason: Option<crate::db_pressure::DbPressureDenyReason>,
) -> bool {
    matches!(
        reason,
        Some(crate::db_pressure::DbPressureDenyReason::PressureCooldown { .. })
    )
}

include!("retention/part-01.rs");
include!("retention/part-02.rs");
include!("retention/part-03.rs");
include!("retention/part-04.rs");
include!("retention/part-05.rs");
include!("retention/part-06.rs");
include!("retention/part-07.rs");
