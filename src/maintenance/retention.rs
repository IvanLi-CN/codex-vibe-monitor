use super::*;

use futures_util::TryStreamExt;
use sqlx::{FromRow, Row};
use std::{
    cell::RefCell,
    fs::File,
    future::Future,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

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
const RETENTION_RECOVERY_LEGACY_SCAN_BATCH: usize = 32;
const RETENTION_RECOVERY_QUARANTINE_GRACE_SECS: i64 = 24 * 60 * 60;
const RETENTION_RECOVERY_STATE_PREPARING: &str = "preparing";
const RETENTION_RECOVERY_STATE_PUBLISHED: &str = "published";
const RETENTION_RECOVERY_STATE_QUARANTINED: &str = "quarantined";

static SYSTEM_TASK_RUN_RETENTION_NEXT_PASS_EPOCH_MS: AtomicU64 = AtomicU64::new(0);
static SYSTEM_TASK_RUN_RETENTION_SCHEDULE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

tokio::task_local! {
    static RETENTION_SHUTDOWN: CancellationToken;
    static RETENTION_CURRENT_PREPARED_KEY: RefCell<Option<String>>;
    static RETENTION_TRY_ARCHIVE_LOCKS: ();
    static RETENTION_RAW_CAPTURE_CIRCUIT: RefCell<Option<Arc<RawCaptureCircuitBreaker>>>;
}

async fn mark_retention_raw_inventory_reset_intent(pool: &Pool<Sqlite>) -> Result<()> {
    let recovery_pending = RETENTION_RAW_CAPTURE_CIRCUIT
        .try_with(|circuit| {
            if let Some(circuit) = circuit.borrow().as_ref() {
                circuit.mark_inventory_preparing()
            } else {
                false
            }
        })
        .unwrap_or(false);
    mark_system_raw_payload_metrics_inventory_reset_pending(pool, 128, recovery_pending).await?;
    Ok(())
}

pub(crate) async fn run_data_retention_maintenance_with_circuit(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: Option<bool>,
    shutdown: Option<&CancellationToken>,
    circuit: Arc<RawCaptureCircuitBreaker>,
) -> Result<RetentionRunSummary> {
    RETENTION_RAW_CAPTURE_CIRCUIT
        .scope(
            RefCell::new(Some(circuit)),
            run_data_retention_maintenance(pool, config, dry_run, shutdown),
        )
        .await
}

pub(crate) async fn retention_try_archive_locks_scope<F: Future>(future: F) -> F::Output {
    RETENTION_TRY_ARCHIVE_LOCKS.scope((), future).await
}

fn retention_archive_locks_are_try_only() -> bool {
    RETENTION_TRY_ARCHIVE_LOCKS.try_with(|_| ()).is_ok()
}

#[cfg(test)]
tokio::task_local! {
    pub(crate) static RETENTION_TEST_WRITE_COORDINATOR:
        std::sync::Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>;
    pub(crate) static RETENTION_TEST_DB_PRESSURE_GATE:
        std::sync::Arc<crate::db_pressure::DbPressureGate>;
    pub(crate) static RETENTION_TEST_LEGACY_DIRECTORY_TRAVERSAL:
        std::sync::Arc<std::sync::atomic::AtomicUsize>;
    pub(crate) static RETENTION_TEST_LEGACY_DIRECTORY_ENTRIES:
        std::sync::Arc<std::sync::atomic::AtomicUsize>;
    pub(crate) static RETENTION_TEST_LEGACY_DIRECTORY_HEAP_LIVE:
        std::sync::Arc<std::sync::atomic::AtomicUsize>;
    pub(crate) static RETENTION_TEST_LEGACY_DIRECTORY_HEAP_PEAK:
        std::sync::Arc<std::sync::atomic::AtomicUsize>;
    pub(crate) static RETENTION_TEST_LEGACY_ARCHIVE_IO:
        std::sync::Arc<std::sync::atomic::AtomicUsize>;
}

#[cfg(test)]
fn retention_test_legacy_directory_traversal_event() {
    let _ = RETENTION_TEST_LEGACY_DIRECTORY_TRAVERSAL.try_with(|counter| {
        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    });
}

#[cfg(test)]
fn retention_test_legacy_directory_entry_event() {
    let _ = RETENTION_TEST_LEGACY_DIRECTORY_ENTRIES.try_with(|counter| {
        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    });
}

#[cfg(test)]
#[derive(Debug, Eq, PartialEq)]
struct RetentionTestLegacyDirectoryHeapEntryGuard {
    observed: bool,
}

#[cfg(test)]
impl RetentionTestLegacyDirectoryHeapEntryGuard {
    fn new() -> Self {
        let observed = RETENTION_TEST_LEGACY_DIRECTORY_HEAP_LIVE
            .try_with(|live| {
                let current = live.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                let _ = RETENTION_TEST_LEGACY_DIRECTORY_HEAP_PEAK.try_with(|peak| {
                    peak.fetch_max(current, std::sync::atomic::Ordering::Relaxed);
                });
            })
            .is_ok();
        Self { observed }
    }
}

#[cfg(test)]
impl Drop for RetentionTestLegacyDirectoryHeapEntryGuard {
    fn drop(&mut self) {
        if !self.observed {
            return;
        }
        let _ = RETENTION_TEST_LEGACY_DIRECTORY_HEAP_LIVE.try_with(|live| {
            live.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        });
    }
}

#[cfg(test)]
fn retention_test_legacy_archive_io_event() {
    let _ = RETENTION_TEST_LEGACY_ARCHIVE_IO.try_with(|counter| {
        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    });
}

static RETENTION_DEFER_GENERATION: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// Advisory directory lock shared by archive publishers and cleanup finalizers. SQLite admission
/// serializes database writers, while this lock also fences their filesystem rename/delete window.
#[cfg(unix)]
pub(crate) struct RetentionArchiveFileLock(Option<File>);

#[cfg(unix)]
impl RetentionArchiveFileLock {
    pub(crate) fn is_held(&self) -> bool {
        self.0.is_some()
    }
}

#[cfg(unix)]
pub(crate) fn retention_archive_file_lock(path: &Path) -> Result<RetentionArchiveFileLock> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("archive path has no parent directory"))?;
    let file = match File::open(parent) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // A cleanup candidate may already have lost its parent directory. There is no
            // filesystem object left for a concurrent publisher to mutate, so let the caller
            // reconcile the metadata under SQLite admission without a directory lock.
            return Ok(RetentionArchiveFileLock(None));
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to open archive directory lock: {}",
                    parent.display()
                )
            });
        }
    };
    let lock_flags = if retention_archive_locks_are_try_only() {
        libc::LOCK_EX | libc::LOCK_NB
    } else {
        libc::LOCK_EX
    };
    let result = unsafe { libc::flock(file.as_raw_fd(), lock_flags) };
    if result != 0 {
        if retention_archive_locks_are_try_only() {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EAGAIN) {
                return Err(anyhow!("archive directory lock busy"));
            }
        }
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("failed to lock archive directory: {}", parent.display()));
    }
    Ok(RetentionArchiveFileLock(Some(file)))
}

pub(crate) fn retention_archive_parent_identity(path: &Path) -> Option<String> {
    let parent = path.parent()?;
    let metadata = fs::metadata(parent).ok()?;
    #[cfg(unix)]
    {
        Some(format!("{}:{}", metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        Some(format!("{}:{:?}", metadata.len(), metadata.modified().ok()))
    }
}

#[cfg(unix)]
impl Drop for RetentionArchiveFileLock {
    fn drop(&mut self) {
        if let Some(file) = self.0.as_ref() {
            let _ = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
        }
    }
}

#[cfg(not(unix))]
pub(crate) struct RetentionArchiveFileLock;

#[cfg(not(unix))]
impl RetentionArchiveFileLock {
    pub(crate) fn is_held(&self) -> bool {
        true
    }
}

#[cfg(not(unix))]
pub(crate) fn retention_archive_file_lock(_path: &Path) -> Result<RetentionArchiveFileLock> {
    Ok(RetentionArchiveFileLock)
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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RetentionRecoveryHealthSnapshot {
    pub(crate) state: String,
    pub(crate) stage: Option<String>,
    pub(crate) prepared_count: Option<usize>,
    pub(crate) quarantined_count: Option<usize>,
    pub(crate) expired_backlog_count: Option<usize>,
    pub(crate) oldest_backlog_age_secs: Option<u64>,
    pub(crate) last_progress_at: Option<String>,
    pub(crate) next_retry_at: Option<String>,
    pub(crate) failure_stage: Option<String>,
    pub(crate) failure_fingerprint: Option<String>,
}

impl Default for RetentionRecoveryHealthSnapshot {
    fn default() -> Self {
        Self {
            state: "unknown".to_string(),
            stage: None,
            prepared_count: None,
            quarantined_count: None,
            expired_backlog_count: None,
            oldest_backlog_age_secs: None,
            last_progress_at: None,
            next_retry_at: None,
            failure_stage: None,
            failure_fingerprint: None,
        }
    }
}

static RETENTION_RECOVERY_HEALTH: Lazy<std::sync::Mutex<RetentionRecoveryHealthSnapshot>> =
    Lazy::new(|| std::sync::Mutex::new(RetentionRecoveryHealthSnapshot::default()));

pub(crate) fn retention_recovery_health_snapshot() -> RetentionRecoveryHealthSnapshot {
    RETENTION_RECOVERY_HEALTH
        .lock()
        .expect("retention recovery health")
        .clone()
}

fn retention_recovery_set_stage(stage: &'static str) {
    let mut health = RETENTION_RECOVERY_HEALTH
        .lock()
        .expect("retention recovery health");
    health.stage = Some(stage.to_string());
    if health.state == "unknown" {
        health.state = "recovering".to_string();
    }
}

fn retention_recovery_record_progress() {
    let mut health = RETENTION_RECOVERY_HEALTH
        .lock()
        .expect("retention recovery health");
    health.state = "healthy".to_string();
    health.last_progress_at = Some(format_utc_iso(Utc::now()));
}

fn retention_recovery_set_current_prepared_key(prepared_key: &str) {
    let _ = RETENTION_CURRENT_PREPARED_KEY.try_with(|current| {
        *current.borrow_mut() = Some(prepared_key.to_string());
    });
}

fn retention_recovery_current_prepared_key() -> Option<String> {
    RETENTION_CURRENT_PREPARED_KEY
        .try_with(|current| current.borrow().clone())
        .ok()
        .flatten()
}

fn retention_recovery_clear_current_prepared_key() {
    let _ = RETENTION_CURRENT_PREPARED_KEY.try_with(|current| {
        *current.borrow_mut() = None;
    });
}

fn retention_recovery_record_failure(stage: &'static str, error: &anyhow::Error) {
    let fingerprint = retention_error_fingerprint(error);
    let mut health = RETENTION_RECOVERY_HEALTH
        .lock()
        .expect("retention recovery health");
    health.state = "degraded".to_string();
    health.failure_stage = Some(stage.to_string());
    health.failure_fingerprint = Some(fingerprint);
}

pub(crate) fn retention_error_fingerprint(error: &anyhow::Error) -> String {
    let digest = Sha256::digest(error.to_string().as_bytes());
    format!("{:x}", digest)[..16].to_string()
}

fn retention_recovery_record_deferred(stage: &'static str) {
    let mut health = RETENTION_RECOVERY_HEALTH
        .lock()
        .expect("retention recovery health");
    health.state = "deferred".to_string();
    health.stage = Some(stage.to_string());
}

fn retention_recovery_log_event(
    level: tracing::Level,
    operation: &'static str,
    message: &'static str,
) {
    let health = retention_recovery_health_snapshot();
    macro_rules! emit {
        ($level:expr) => {
            tracing::event!(
                $level,
                operation = operation,
                message = message,
                retention_recovery_state = %health.state,
                retention_recovery_stage = ?health.stage,
                retention_recovery_prepared_count = ?health.prepared_count,
                retention_recovery_quarantined_count = ?health.quarantined_count,
                retention_recovery_expired_backlog_count = ?health.expired_backlog_count,
                retention_recovery_oldest_backlog_age_secs = ?health.oldest_backlog_age_secs,
                retention_recovery_last_progress_at = ?health.last_progress_at,
                retention_recovery_next_retry_at = ?health.next_retry_at,
                retention_recovery_failure_stage = ?health.failure_stage,
                retention_recovery_failure_fingerprint = ?health.failure_fingerprint,
                "retention recovery diagnostics"
            )
        };
    }
    match level {
        tracing::Level::ERROR => emit!(tracing::Level::ERROR),
        tracing::Level::WARN => emit!(tracing::Level::WARN),
        tracing::Level::INFO => emit!(tracing::Level::INFO),
        tracing::Level::DEBUG => emit!(tracing::Level::DEBUG),
        tracing::Level::TRACE => emit!(tracing::Level::TRACE),
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

#[derive(Debug)]
struct RetentionRecoveryFailurePersisted {
    prepared_key: String,
    source: anyhow::Error,
}

impl std::fmt::Display for RetentionRecoveryFailurePersisted {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let category = self.source.to_string();
        let category = if category.contains("source identity verification failed") {
            "source identity verification failed"
        } else if category.contains("artifact digest verification failed") {
            "artifact digest verification failed"
        } else {
            "retention operation failed"
        };
        write!(
            formatter,
            "retention recovery failure persisted: {category}"
        )
    }
}

impl std::error::Error for RetentionRecoveryFailurePersisted {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

pub(crate) fn retention_recovery_failure_persisted(
    prepared_key: &str,
    source: anyhow::Error,
) -> anyhow::Error {
    anyhow::Error::new(RetentionRecoveryFailurePersisted {
        prepared_key: prepared_key.to_string(),
        source,
    })
}

pub(crate) fn is_retention_recovery_failure_persisted(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<RetentionRecoveryFailurePersisted>()
        .is_some_and(|failure| !failure.prepared_key.is_empty())
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
    let mut health = RETENTION_WRITE_HEALTH
        .lock()
        .expect("retention write health");
    let breached = observe_retention_write_commit(
        &mut health,
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
    );
    drop(health);
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

#[allow(clippy::too_many_arguments)]
fn observe_retention_write_commit(
    health: &mut RetentionWriteHealthState,
    operation: &'static str,
    admission_mode: &'static str,
    rows: usize,
    estimated_bytes: usize,
    prepare_elapsed: Duration,
    lock_wait: Duration,
    execute_elapsed: Duration,
    commit_elapsed: Duration,
    p1_waiter_count: usize,
    candidate_remaining_hint: usize,
) -> bool {
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
    health.snapshot.last_error = Some(format!(
        "failure_fingerprint:{}",
        retention_error_fingerprint(error)
    ));
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
    #[cfg(test)]
    let test_pressure_gate = RETENTION_TEST_DB_PRESSURE_GATE
        .try_with(std::sync::Arc::clone)
        .ok();
    #[cfg(not(test))]
    let test_pressure_gate: Option<std::sync::Arc<crate::db_pressure::DbPressureGate>> = None;
    let pressure_gate = match test_pressure_gate.as_deref() {
        Some(gate) => gate,
        None => crate::db_pressure::global_db_pressure_gate(),
    };
    if let Some(reason) = pressure_gate.background_deny_reason() {
        retention_record_defer(operation, reason);
        return None;
    }
    #[cfg(test)]
    let coordinator = RETENTION_TEST_WRITE_COORDINATOR
        .try_with(std::sync::Arc::clone)
        .unwrap_or_else(|_| {
            crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        });
    #[cfg(not(test))]
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
    let coordinator_snapshot = coordinator.snapshot().await;
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

pub(crate) async fn prune_system_task_runs(pool: &Pool<Sqlite>, dry_run: bool) -> Result<usize> {
    let now_epoch_ms = system_task_run_retention_now_epoch_ms();
    if !system_task_run_retention_pass_is_due(now_epoch_ms) {
        return Ok(0);
    }

    let success_cutoff = format_utc_iso_millis(Utc::now() - ChronoDuration::days(30));
    let failed_cutoff = format_utc_iso_millis(Utc::now() - ChronoDuration::days(180));
    if dry_run {
        let candidates: i64 = sqlx::query_scalar(
            r#"
            WITH ranked AS (
                SELECT
                    task_kind,
                    status,
                    started_at,
                    ROW_NUMBER() OVER (
                        PARTITION BY task_kind, status
                        ORDER BY started_at DESC, id DESC
                    ) AS retention_rank
                FROM system_task_runs
                WHERE status IN ('success', 'skipped', 'failed')
                  AND strftime('%Y-%m-%dT%H:%M:%fZ', started_at) = started_at
            )
            SELECT COUNT(*)
            FROM ranked
            WHERE retention_rank > ?1
              AND (
                  (status IN ('success', 'skipped')
                    AND (started_at < ?2 OR retention_rank > 5000))
                  OR (status = 'failed'
                    AND (started_at < ?3 OR retention_rank > 10000))
              )
            "#,
        )
        .bind(SYSTEM_TASK_RUN_RETENTION_KEEP_RECENT)
        .bind(&success_cutoff)
        .bind(&failed_cutoff)
        .fetch_one(pool)
        .await
        .map_err(anyhow::Error::from)
        .or_else(|error| {
            if system_task_run_retention_handle_pressure(&error) {
                Ok(0)
            } else {
                Err(error.context("failed to count system task retention candidates"))
            }
        })?;
        return Ok((candidates.max(0) as usize).min(SYSTEM_TASK_RUN_RETENTION_MAX_ROWS_PER_PASS));
    }
    let candidates = match sqlx::query_as::<_, SystemTaskRunRetentionCandidate>(
        r#"
            WITH ranked AS (
                SELECT
                    id,
                    task_kind,
                    status,
                    started_at,
                    ROW_NUMBER() OVER (
                        PARTITION BY task_kind, status
                        ORDER BY started_at DESC, id DESC
                    ) AS retention_rank
                FROM system_task_runs
                WHERE status IN ('success', 'skipped', 'failed')
                  AND strftime('%Y-%m-%dT%H:%M:%fZ', started_at) = started_at
            )
            SELECT id
            FROM ranked
            WHERE retention_rank > ?1
              AND (
                  (status IN ('success', 'skipped')
                    AND (started_at < ?2 OR retention_rank > 5000))
                  OR (status = 'failed'
                    AND (started_at < ?3 OR retention_rank > 10000))
              )
            ORDER BY started_at ASC, id ASC
            LIMIT ?4
            "#,
    )
    .bind(SYSTEM_TASK_RUN_RETENTION_KEEP_RECENT)
    .bind(&success_cutoff)
    .bind(&failed_cutoff)
    .bind(SYSTEM_TASK_RUN_RETENTION_MAX_ROWS_PER_PASS as i64)
    .fetch_all(pool)
    .await
    {
        Ok(candidates) => candidates,
        Err(error) => {
            let error = anyhow::Error::from(error);
            if system_task_run_retention_handle_pressure(&error) {
                return Ok(0);
            }
            return Err(error).context("failed to select system task retention candidates");
        }
    };
    if candidates.is_empty() {
        return Ok(0);
    }

    let mut pruned = 0usize;
    for candidates in candidates.chunks(SYSTEM_TASK_RUN_RETENTION_TERMINAL_BATCH_ROWS) {
        let Some(admission) = acquire_retention_write_admission("system_task_run_retention").await
        else {
            // Admission can be denied before a SQL call observes the pressure error. A
            // cooldown is distinct from shutdown or a normally busy background slot.
            if system_task_run_retention_admission_requires_pressure_backoff(
                crate::db_pressure::global_db_pressure_gate().background_deny_reason(),
            ) {
                system_task_run_retention_schedule_next(
                    system_task_run_retention_now_epoch_ms(),
                    SYSTEM_TASK_RUN_RETENTION_PRESSURE_BACKOFF,
                );
            }
            break;
        };
        let execute_started = Instant::now();
        let ids = candidates
            .iter()
            .map(|candidate| candidate.id)
            .collect::<Vec<_>>();
        let mut delete = QueryBuilder::<Sqlite>::new("DELETE FROM system_task_runs WHERE id IN (");
        let mut separated = delete.separated(", ");
        for id in &ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
        let mut transaction = match pool.begin().await {
            Ok(transaction) => transaction,
            Err(error) => {
                let error = anyhow::Error::from(error);
                if system_task_run_retention_handle_pressure(&error) {
                    return Ok(pruned);
                }
                return Err(error).context("failed to begin system task retention batch");
            }
        };
        let deleted = match delete.build().execute(&mut *transaction).await {
            Ok(result) => result.rows_affected() as usize,
            Err(error) => {
                let error = anyhow::Error::from(error);
                if system_task_run_retention_handle_pressure(&error) {
                    return Ok(pruned);
                }
                return Err(error).context("failed to delete system task retention batch");
            }
        };
        if let Err(error) = transaction.commit().await {
            let error = anyhow::Error::from(error);
            if system_task_run_retention_handle_pressure(&error) {
                return Ok(pruned);
            }
            return Err(error).context("failed to commit system task retention batch");
        }
        retention_record_commit!(
            "system_task_run_retention",
            admission.admission_mode(),
            deleted,
            deleted.saturating_mul(128),
            Duration::ZERO,
            admission.lock_wait(),
            execute_started.elapsed(),
            Duration::ZERO,
            admission.p1_waiter_count(),
            usize::from(candidates.len() == SYSTEM_TASK_RUN_RETENTION_TERMINAL_BATCH_ROWS),
        );
        pruned += deleted;
        if deleted < candidates.len() {
            break;
        }
    }

    Ok(pruned)
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

async fn load_invocation_archive_candidates_by_ids(
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

#[derive(Debug, Clone)]
struct RetentionPreparedArchiveDescriptor {
    prepared_key: String,
    dataset: &'static str,
    month_key: String,
    day_key: Option<String>,
    part_key: Option<String>,
    file_path: String,
    source_ids_json: String,
    source_identity_sha256: String,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum InvocationArchiveIdentityDatabase {
    Main,
    Archive,
}

impl InvocationArchiveIdentityDatabase {
    fn table_name(self) -> &'static str {
        match self {
            Self::Main => "main.codex_invocations",
            Self::Archive => "archive_db.codex_invocations",
        }
    }
}

fn hash_identity_component(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

pub(crate) async fn invocation_archive_source_identity_sha256(
    connection: &mut sqlx::SqliteConnection,
    database: InvocationArchiveIdentityDatabase,
    ids: &[i64],
) -> Result<String> {
    if ids.is_empty() {
        bail!("retention archive source identity requires at least one row");
    }

    let columns = CODEX_INVOCATIONS_ARCHIVE_COLUMNS
        .split(", ")
        .collect::<Vec<_>>();
    let table_name = database.table_name();
    let mut query = sqlx::QueryBuilder::<Sqlite>::new("SELECT ");
    for (index, column) in columns.iter().enumerate() {
        if index > 0 {
            query.push(", ");
        }
        query
            .push("typeof(")
            .push(table_name)
            .push(".")
            .push(*column)
            .push("), CAST(")
            .push(table_name)
            .push(".")
            .push(*column)
            .push(" AS BLOB)");
    }
    query
        .push(" FROM ")
        .push(table_name)
        .push(" WHERE id IN (SELECT value FROM json_each(")
        .push_bind(serde_json::to_string(ids).context("encode retention archive identity ids")?)
        .push(")) ORDER BY id ASC");

    // Length framing and SQLite storage classes keep NULL, text, numeric, and blob values distinct.
    let mut hasher = Sha256::new();
    hasher.update(b"codex-vibe-monitor/retention-source-identity/v2\0");
    hasher.update((ids.len() as u64).to_be_bytes());
    let mut row_count = 0usize;
    let mut rows = query.build().fetch(&mut *connection);
    while let Some(row) = rows.try_next().await? {
        row_count += 1;
        for (index, column) in columns.iter().enumerate() {
            let sqlite_type = row.try_get::<String, _>(index * 2)?;
            let value = row.try_get::<Option<Vec<u8>>, _>(index * 2 + 1)?;
            hash_identity_component(&mut hasher, column.as_bytes());
            hash_identity_component(&mut hasher, sqlite_type.as_bytes());
            match value {
                Some(value) => {
                    hasher.update([1]);
                    hash_identity_component(&mut hasher, &value);
                }
                None => hasher.update([0]),
            }
        }
        hasher.update([0xff]);
    }

    if row_count != ids.len() {
        bail!("retention archive source identity verification failed: source row count changed");
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn retention_prepared_archive_descriptor(
    config: &AppConfig,
    dataset: &'static str,
    group_key: &str,
    candidates: &[InvocationArchiveCandidate],
    source_identity_sha256: String,
) -> Result<RetentionPreparedArchiveDescriptor> {
    let mut ids = candidates
        .iter()
        .map(|candidate| candidate.id)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        bail!("retention prepared archive requires source ids");
    }
    let source_ids_json =
        serde_json::to_string(&ids).context("encode retention prepared archive source ids")?;
    let layout = archive_layout_for_dataset(config, dataset);
    let (month_key, day_key, part_key, file_path) = match layout {
        ArchiveBatchLayout::LegacyMonth => {
            let file_path = archive_batch_file_path(config, dataset, group_key)?;
            (
                group_key.to_string(),
                None,
                None,
                file_path.to_string_lossy().to_string(),
            )
        }
        ArchiveBatchLayout::SegmentV1 => {
            let part_key = archive_segment_part_key_for_ids(&ids)?;
            let file_path = archive_segment_file_path(
                config,
                dataset,
                group_key,
                &part_key,
                config.invocation_archive_codec,
            )?;
            (
                archive_month_key_from_day_key(group_key)?,
                Some(group_key.to_string()),
                Some(part_key),
                file_path.to_string_lossy().to_string(),
            )
        }
    };
    Ok(RetentionPreparedArchiveDescriptor {
        prepared_key: format!("{dataset}:{file_path}:{source_identity_sha256}"),
        dataset,
        month_key,
        day_key,
        part_key,
        file_path,
        source_ids_json,
        source_identity_sha256,
    })
}

fn retention_live_mirror_archive_path(
    config: &AppConfig,
    group_key: &str,
    ids: &[i64],
    source_identity_sha256: &str,
) -> Result<PathBuf> {
    match archive_layout_for_dataset(config, "codex_invocations") {
        ArchiveBatchLayout::LegacyMonth => {
            let standard_path = archive_batch_file_path(config, "codex_invocations", group_key)?;
            let file_name = format!(
                "codex_invocations-{group_key}-live-mirror-{}.sqlite.gz",
                &source_identity_sha256[..source_identity_sha256.len().min(16)]
            );
            Ok(standard_path.with_file_name(file_name))
        }
        ArchiveBatchLayout::SegmentV1 => {
            let part_key = archive_segment_part_key_for_ids(ids)?;
            let standard_path = archive_segment_file_path(
                config,
                "codex_invocations",
                group_key,
                &part_key,
                config.invocation_archive_codec,
            )?;
            Ok(standard_path.with_file_name(format!(
                "{part_key}.live-mirror.sqlite.{}",
                config.invocation_archive_codec.file_extension()
            )))
        }
    }
}

async fn retention_recovery_refresh_counts(pool: &Pool<Sqlite>, config: &AppConfig) -> Result<()> {
    let (prepared_count, quarantined_count, next_retry_at) =
        sqlx::query_as::<_, (i64, i64, Option<String>)>(
            r#"
        SELECT
            COALESCE(SUM(CASE WHEN state IN ('preparing', 'published') THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN state = 'quarantined' THEN 1 ELSE 0 END), 0),
            MIN(next_retry_at)
        FROM retention_prepared_archives
        "#,
        )
        .fetch_one(pool)
        .await?;
    let cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let (expired_backlog_count, oldest_backlog_at) = sqlx::query_as::<_, (i64, Option<String>)>(
        r#"
            SELECT COUNT(*), MIN(occurred_at)
            FROM codex_invocations
            WHERE occurred_at < ?1
            "#,
    )
    .bind(cutoff)
    .fetch_one(pool)
    .await?;
    let now = Utc::now();
    let oldest_backlog_age_secs = oldest_backlog_at
        .as_deref()
        .and_then(parse_to_utc_datetime)
        .map(|occurred_at| now.signed_duration_since(occurred_at).num_seconds().max(0) as u64);
    let mut health = RETENTION_RECOVERY_HEALTH
        .lock()
        .expect("retention recovery health");
    health.prepared_count = Some(prepared_count.max(0) as usize);
    health.quarantined_count = Some(quarantined_count.max(0) as usize);
    health.expired_backlog_count = Some(expired_backlog_count.max(0) as usize);
    health.oldest_backlog_age_secs = oldest_backlog_age_secs;
    health.next_retry_at = next_retry_at;
    if health.quarantined_count.is_some_and(|count| count > 0) {
        health.state = "degraded".to_string();
    } else if health.prepared_count.is_some_and(|count| count > 0)
        || health.expired_backlog_count.is_some_and(|count| count > 0)
    {
        if !matches!(health.state.as_str(), "degraded" | "deferred") {
            health.state = "recovering".to_string();
        }
    } else if health.prepared_count == Some(0)
        && health.quarantined_count == Some(0)
        && health.expired_backlog_count == Some(0)
        && matches!(
            health.state.as_str(),
            "unknown" | "recovering" | "deferred" | "degraded"
        )
    {
        health.state = "healthy".to_string();
        health.failure_stage = None;
        health.failure_fingerprint = None;
    }
    Ok(())
}

async fn retention_recovery_record_preparing(
    pool: &Pool<Sqlite>,
    descriptor: &RetentionPreparedArchiveDescriptor,
) -> Result<()> {
    retention_recovery_set_stage("preparing");
    let Some(admission) = acquire_retention_write_admission("retention_recovery_prepare").await
    else {
        return Err(retention_write_deferred("retention_recovery_prepare"));
    };
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let existing = sqlx::query_as::<_, (String, String)>(
        "SELECT prepared_key, source_identity_sha256 FROM retention_prepared_archives WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(descriptor.dataset)
    .bind(&descriptor.file_path)
    .fetch_optional(tx.as_mut())
    .await?;
    if let Some((existing_key, existing_identity)) = existing
        && existing_identity != descriptor.source_identity_sha256
    {
        tx.rollback().await?;
        drop(admission);
        let error = anyhow!("retention prepared archive identity collision");
        retention_recovery_persist_failure(pool, &existing_key, "preparing", &error).await?;
        return Err(retention_recovery_failure_persisted(&existing_key, error));
    }
    sqlx::query(
        r#"
        INSERT INTO retention_prepared_archives (
            prepared_key,
            dataset,
            month_key,
            day_key,
            part_key,
            file_path,
            source_ids_json,
            source_identity_sha256,
            state,
            attempt_count,
            next_retry_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, NULL)
        ON CONFLICT(dataset, file_path) DO UPDATE SET
            prepared_key = excluded.prepared_key,
            dataset = excluded.dataset,
            month_key = excluded.month_key,
            day_key = excluded.day_key,
            part_key = excluded.part_key,
            file_path = excluded.file_path,
            source_ids_json = excluded.source_ids_json,
            source_identity_sha256 = excluded.source_identity_sha256,
            state = CASE
                WHEN retention_prepared_archives.state = 'quarantined'
                    THEN retention_prepared_archives.state
                ELSE excluded.state
            END,
            attempt_count = retention_prepared_archives.attempt_count + 1,
            updated_at = datetime('now')
        WHERE retention_prepared_archives.next_retry_at IS NULL
           OR retention_prepared_archives.next_retry_at <= datetime('now')
        "#,
    )
    .bind(&descriptor.prepared_key)
    .bind(descriptor.dataset)
    .bind(&descriptor.month_key)
    .bind(&descriptor.day_key)
    .bind(&descriptor.part_key)
    .bind(&descriptor.file_path)
    .bind(&descriptor.source_ids_json)
    .bind(&descriptor.source_identity_sha256)
    .bind(RETENTION_RECOVERY_STATE_PREPARING)
    .execute(tx.as_mut())
    .await?;
    let (state, retry_due): (String, i64) = sqlx::query_as(
        r#"
        SELECT state,
               CASE WHEN next_retry_at IS NOT NULL AND next_retry_at > datetime('now') THEN 1 ELSE 0 END
        FROM retention_prepared_archives
        WHERE prepared_key = ?1
        "#,
    )
    .bind(&descriptor.prepared_key)
    .fetch_one(tx.as_mut())
    .await?
    ;
    if state == RETENTION_RECOVERY_STATE_QUARANTINED || retry_due != 0 {
        tx.rollback().await?;
        return Err(retention_write_deferred("retention_recovery_backoff"));
    }
    tx.commit().await?;
    drop(admission);
    Ok(())
}

pub(crate) async fn retention_recovery_persist_failure(
    pool: &Pool<Sqlite>,
    prepared_key: &str,
    stage: &'static str,
    error: &anyhow::Error,
) -> Result<()> {
    retention_recovery_record_failure(stage, error);
    let fingerprint = retention_recovery_health_snapshot()
        .failure_fingerprint
        .unwrap_or_default();
    let quarantine = error.to_string().contains("identity collision")
        || error.to_string().contains("verification failed");
    let Some(admission) = acquire_retention_write_admission("retention_recovery_failure").await
    else {
        retention_recovery_record_deferred(stage);
        return Err(retention_write_deferred("retention_recovery_failure"));
    };
    let execute_started = Instant::now();
    let observed_artifact_sha256 = sqlx::query_scalar::<_, Option<String>>(
        "SELECT artifact_sha256 FROM retention_prepared_archives WHERE prepared_key = ?1",
    )
    .bind(prepared_key)
    .fetch_optional(pool)
    .await?
    .flatten();
    let attempt_count = sqlx::query_scalar::<_, i64>(
        "SELECT attempt_count FROM retention_prepared_archives WHERE prepared_key = ?1",
    )
    .bind(prepared_key)
    .fetch_optional(pool)
    .await?
    .unwrap_or(0)
    .max(0) as u32;
    let retry_seconds = 30_u64
        .saturating_mul(1_u64 << attempt_count.min(7))
        .min(3_600);
    // Preserve the first quarantine timestamp so repeated identity collisions cannot postpone
    // the bounded cleanup window forever.
    sqlx::query(
        r#"
        UPDATE retention_prepared_archives
        SET state = CASE WHEN ?1 THEN ?2 ELSE state END,
            quarantined_at = CASE
                WHEN ?1 AND quarantined_at IS NULL THEN datetime('now')
                ELSE quarantined_at
            END,
            last_failure_stage = ?3,
            last_failure_fingerprint = ?4,
            next_retry_at = CASE WHEN ?1 THEN NULL ELSE datetime('now', ?5) END,
            attempt_count = attempt_count + 1,
            updated_at = datetime('now')
        WHERE prepared_key = ?6
          AND (
              state = 'preparing'
              OR (
                  state = 'published'
                  AND (
                      artifact_sha256 IS NULL
                      OR ?7 IN ('finalizing', 'publishing', 'legacy_reconcile')
                  )
              )
          )
          AND artifact_sha256 IS ?8
        "#,
    )
    .bind(quarantine)
    .bind(RETENTION_RECOVERY_STATE_QUARANTINED)
    .bind(stage)
    .bind(fingerprint)
    .bind(format!("+{retry_seconds} seconds"))
    .bind(prepared_key)
    .bind(stage)
    .bind(observed_artifact_sha256.as_deref())
    .execute(pool)
    .await?;
    retention_record_commit!(
        "retention_recovery_failure",
        admission.admission_mode(),
        1,
        128,
        Duration::ZERO,
        admission.lock_wait(),
        execute_started.elapsed(),
        Duration::ZERO,
        admission.p1_waiter_count,
        0,
    );
    drop(admission);
    Ok(())
}

async fn retention_recovery_persist_latest_failure_best_effort(
    pool: &Pool<Sqlite>,
    stage: &'static str,
    error: &anyhow::Error,
) {
    let persist_result = async {
        let prepared_key = retention_recovery_current_prepared_key();
        if let Some(prepared_key) = prepared_key {
            retention_recovery_persist_failure(pool, &prepared_key, stage, error).await?;
        } else {
            retention_recovery_record_failure(stage, error);
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;

    if let Err(persist_error) = persist_result {
        if is_retention_write_deferred(&persist_error) {
            retention_recovery_record_deferred(stage);
        } else {
            retention_recovery_record_failure(stage, error);
        }
        retention_recovery_log_event(
            tracing::Level::WARN,
            "failure_persist",
            "retention recovery failure state could not be persisted; continuing independent stages",
        );
    }
}

async fn retention_recovery_mark_published(
    pool: &Pool<Sqlite>,
    descriptor: &RetentionPreparedArchiveDescriptor,
    artifact_sha256: &str,
) -> Result<()> {
    retention_recovery_set_stage("publishing");
    let artifact_bytes = fs::metadata(&descriptor.file_path)
        .context("failed to inspect published archive artifact")?
        .len();
    let Some(admission) = acquire_retention_write_admission("retention_recovery_publish").await
    else {
        return Err(retention_write_deferred("retention_recovery_publish"));
    };
    let updated = sqlx::query(
        r#"
        UPDATE retention_prepared_archives
        SET state = ?1,
            artifact_sha256 = ?2,
            artifact_bytes = ?3,
            next_retry_at = NULL,
            last_failure_stage = NULL,
            last_failure_fingerprint = NULL,
            updated_at = datetime('now')
        WHERE prepared_key = ?4
          AND state = 'preparing'
          AND staged_file_path IS NULL
        "#,
    )
    .bind(RETENTION_RECOVERY_STATE_PUBLISHED)
    .bind(artifact_sha256)
    .bind(artifact_bytes as i64)
    .bind(&descriptor.prepared_key)
    .execute(pool)
    .await?;
    drop(admission);
    if updated.rows_affected() == 0 {
        return Err(anyhow!(
            "prepared archive journal changed during publication"
        ));
    }
    Ok(())
}

async fn retention_recovery_verify_publication_tx(
    tx: &mut sqlx::SqliteConnection,
    descriptor: &RetentionPreparedArchiveDescriptor,
    artifact_sha256: &str,
) -> Result<()> {
    let journal_identity = sqlx::query_scalar::<_, String>(
        "SELECT source_identity_sha256 FROM retention_prepared_archives WHERE prepared_key = ?1 AND state = 'published'",
    )
    .bind(&descriptor.prepared_key)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| anyhow!("published retention archive journal entry is missing"))?;
    if journal_identity != descriptor.source_identity_sha256 {
        bail!("retention prepared archive source identity verification failed");
    }
    let expected_sha = sqlx::query_scalar::<_, Option<String>>(
        "SELECT artifact_sha256 FROM retention_prepared_archives WHERE prepared_key = ?1",
    )
    .bind(&descriptor.prepared_key)
    .fetch_one(&mut *tx)
    .await?
    .ok_or_else(|| anyhow!("retention prepared archive artifact digest is missing"))?;
    if expected_sha != artifact_sha256 {
        bail!("retention prepared archive artifact digest verification failed");
    }
    let source_ids = serde_json::from_str::<Vec<i64>>(&descriptor.source_ids_json)
        .context("decode retention prepared archive source ids")?;
    let source_identity = invocation_archive_source_identity_sha256(
        tx,
        InvocationArchiveIdentityDatabase::Main,
        &source_ids,
    )
    .await?;
    if source_identity != journal_identity {
        bail!(
            "retention prepared archive source identity verification failed: live rows no longer match journal"
        );
    }
    Ok(())
}

async fn verify_prepared_retention_archive_artifact(
    pool: &Pool<Sqlite>,
    descriptor: &RetentionPreparedArchiveDescriptor,
    archive_path: &Path,
    expected_sha256: &str,
) -> Result<()> {
    if descriptor.dataset != "codex_invocations" {
        bail!("unsupported prepared archive dataset for recovery verification");
    }
    let temp_path = PathBuf::from(format!(
        "{}.{}.recovery.sqlite",
        archive_path.display(),
        retention_temp_suffix()
    ));
    let _temp_cleanup = TempSqliteCleanup(temp_path.clone());
    inflate_gzip_sqlite_file(archive_path, &temp_path)?;
    let mut archive_db = open_archive_sqlite_connection(&temp_path).await?;
    ensure_codex_invocations_archive_schema_direct(&mut archive_db).await?;
    let archive_ids =
        sqlx::query_scalar::<_, i64>("SELECT id FROM codex_invocations ORDER BY id ASC")
            .fetch_all(&mut archive_db)
            .await?;
    let expected_ids = serde_json::from_str::<Vec<i64>>(&descriptor.source_ids_json)?;
    if descriptor.part_key.is_some() && archive_ids != expected_ids {
        bail!("prepared archive source identity verification failed");
    }
    if descriptor.part_key.is_none()
        && expected_ids
            .iter()
            .any(|expected_id| archive_ids.binary_search(expected_id).is_err())
    {
        bail!("prepared archive source identity verification failed");
    }
    let identity_ids = if descriptor.part_key.is_some() {
        archive_ids.as_slice()
    } else {
        expected_ids.as_slice()
    };
    let archive_identity = invocation_archive_source_identity_sha256(
        &mut archive_db,
        InvocationArchiveIdentityDatabase::Main,
        identity_ids,
    )
    .await?;
    archive_db.close().await?;
    if archive_identity != descriptor.source_identity_sha256 {
        bail!("prepared archive source identity verification failed");
    }
    let mut source_connection = pool.acquire().await?;
    let source_identity = invocation_archive_source_identity_sha256(
        &mut source_connection,
        InvocationArchiveIdentityDatabase::Main,
        &expected_ids,
    )
    .await?;
    drop(source_connection);
    if source_identity != descriptor.source_identity_sha256 {
        bail!("prepared archive live source identity verification failed");
    }
    if sha256_hex_file(archive_path)? != expected_sha256 {
        bail!("prepared archive artifact digest verification failed");
    }
    Ok(())
}

async fn retention_recovery_delete_tx(
    tx: &mut sqlx::SqliteConnection,
    prepared_key: &str,
) -> Result<()> {
    sqlx::query("DELETE FROM retention_prepared_archives WHERE prepared_key = ?1")
        .bind(prepared_key)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

async fn quarantine_published_retention_archive_if_unchanged(
    pool: &Pool<Sqlite>,
    prepared_key: &str,
    expected_artifact_sha256: Option<&str>,
    failure_fingerprint: Option<&str>,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE retention_prepared_archives
        SET state = ?1,
            quarantined_at = datetime('now'),
            last_failure_stage = ?2,
            last_failure_fingerprint = ?3,
            updated_at = datetime('now')
        WHERE prepared_key = ?4
          AND state = ?5
          AND artifact_sha256 IS ?6
        "#,
    )
    .bind(RETENTION_RECOVERY_STATE_QUARANTINED)
    .bind("legacy_reconcile")
    .bind(failure_fingerprint)
    .bind(prepared_key)
    .bind(RETENTION_RECOVERY_STATE_PUBLISHED)
    .bind(expected_artifact_sha256)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub(crate) fn retention_archive_path_is_within_root(config: &AppConfig, path: &Path) -> bool {
    retention_archive_path_is_within(
        &resolved_archive_dir(config).join("codex_invocations"),
        path,
    )
}

pub(crate) fn retention_archive_path_is_within_archive_root(
    config: &AppConfig,
    path: &Path,
) -> bool {
    retention_archive_path_is_within(&resolved_archive_dir(config), path)
}

fn retention_archive_path_is_within(root: &Path, path: &Path) -> bool {
    let Ok(root) = fs::canonicalize(root) else {
        return false;
    };
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return false;
    }
    let candidates = if path.is_absolute() {
        vec![path.to_path_buf()]
    } else {
        vec![path.to_path_buf(), root.join(path)]
    };
    candidates.into_iter().any(|candidate| {
        let mut current = candidate.as_path();
        loop {
            if let Ok(canonical) = fs::canonicalize(current) {
                return canonical.starts_with(&root);
            }
            current = match current.parent() {
                Some(parent) if parent != current => parent,
                _ => return false,
            };
        }
    })
}

pub(crate) fn retention_archive_path_is_owned(config: &AppConfig, path: &Path) -> bool {
    let Ok(root) = fs::canonicalize(resolved_archive_dir(config).join("codex_invocations")) else {
        return false;
    };
    if !retention_archive_path_is_within_root(config, path) {
        return false;
    }
    let candidates = if path.is_absolute() {
        vec![path.to_path_buf()]
    } else {
        vec![path.to_path_buf(), root.join(path)]
    };
    candidates.into_iter().any(|candidate| {
        let Ok(metadata) = fs::symlink_metadata(&candidate) else {
            return false;
        };
        if !metadata.file_type().is_file() {
            return false;
        }
        let Ok(canonical_candidate) = fs::canonicalize(&candidate) else {
            return false;
        };
        canonical_candidate.starts_with(&root)
    })
}

async fn reconcile_retention_prepared_archives(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<()> {
    retention_recovery_set_stage("legacy_reconcile");
    let rows = sqlx::query_as::<
        _,
        (
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            String,
            Option<String>,
            Option<String>,
            String,
            String,
            Option<String>,
        ),
    >(
        r#"
        SELECT prepared_key, file_path, state, artifact_sha256, quarantined_at,
               month_key, day_key, part_key, source_ids_json, source_identity_sha256,
               staged_file_path
        FROM retention_prepared_archives
        WHERE (
            state IN ('preparing', 'published')
            AND (next_retry_at IS NULL OR next_retry_at <= datetime('now'))
        ) OR (
            state = 'quarantined'
            AND quarantined_at IS NOT NULL
            AND julianday('now') - julianday(quarantined_at) >= (?1 / 86400.0)
            AND NOT EXISTS (
                SELECT 1 FROM archive_batches
                WHERE archive_batches.file_path = retention_prepared_archives.file_path
            )
            AND NOT EXISTS (
                SELECT 1 FROM retention_prepared_archives AS active
                WHERE active.file_path = retention_prepared_archives.file_path
                  AND active.prepared_key <> retention_prepared_archives.prepared_key
                  AND active.state IN ('preparing', 'published')
            )
        )
        ORDER BY CASE
                     WHEN state = 'published' THEN 0
                     WHEN state = 'quarantined' THEN 1
                     ELSE 2
                 END,
                 updated_at ASC, prepared_key ASC
        LIMIT ?2
        "#,
    )
    .bind(RETENTION_RECOVERY_QUARANTINE_GRACE_SECS)
    .bind(RETENTION_RECOVERY_LEGACY_SCAN_BATCH as i64)
    .fetch_all(pool)
    .await?;
    for (
        prepared_key,
        file_path,
        state,
        artifact_sha256,
        quarantined_at,
        month_key,
        day_key,
        part_key,
        source_ids_json,
        source_identity_sha256,
        staged_file_path,
    ) in rows
    {
        let path = Path::new(&file_path);
        if !retention_archive_path_is_within_root(config, path) {
            if state == RETENTION_RECOVERY_STATE_PREPARING
                || state == RETENTION_RECOVERY_STATE_PUBLISHED
            {
                let error = anyhow!("prepared archive path failed ownership verification");
                retention_recovery_persist_failure(pool, &prepared_key, "legacy_reconcile", &error)
                    .await?;
            }
            continue;
        }
        if staged_file_path.is_some() {
            // The restore decision and the cleanup of its journal pointer must share the same
            // directory fence as the publisher. A pre-lock snapshot could otherwise overwrite a
            // newer retry artifact or clear a newly written staging path.
            let _archive_lock = retention_archive_file_lock(path)?;
            let Some((current_state, Some(staged_file_path))) =
                sqlx::query_as::<_, (String, Option<String>)>(
                    "SELECT state, staged_file_path FROM retention_prepared_archives WHERE prepared_key = ?1",
                )
                .bind(&prepared_key)
                .fetch_optional(pool)
                .await?
            else {
                continue;
            };
            let state = current_state;
            let staged_path = Path::new(&staged_file_path);
            if !retention_archive_path_is_within_root(config, staged_path)
                || !replacement_staging_path_is_owned(path, staged_path)
            {
                let error = anyhow!("prepared archive staging path failed ownership verification");
                retention_recovery_persist_failure(pool, &prepared_key, "legacy_reconcile", &error)
                    .await?;
                continue;
            }
            let archive_staged_path = sqlx::query_scalar::<_, Option<String>>(
                "SELECT replacement_staged_path FROM archive_batches
                 WHERE file_path = ?1 ORDER BY id DESC LIMIT 1",
            )
            .bind(&file_path)
            .fetch_optional(pool)
            .await?
            .flatten();
            if archive_staged_path.as_deref() != Some(staged_file_path.as_str()) {
                // The prepared pointer was durable but the archive manifest pointer was not;
                // no rename could have started, so clear the orphaned pointer and retry.
                clear_retention_staged_file_path(pool, &prepared_key, &staged_file_path, true)
                    .await?;
                continue;
            }
            let manifest_sha = sqlx::query_scalar::<_, Option<String>>(
                "SELECT sha256 FROM archive_batches WHERE file_path = ?1 ORDER BY id DESC LIMIT 1",
            )
            .bind(&file_path)
            .fetch_optional(pool)
            .await?
            .flatten();
            let current_sha = if path.is_file() {
                Some(sha256_hex_file(path)?)
            } else {
                None
            };
            if manifest_sha.is_some() && manifest_sha == current_sha {
                if staged_path.is_file() {
                    fs::remove_file(staged_path)
                        .context("failed to remove committed legacy archive rollback copy")?;
                }
                if state == RETENTION_RECOVERY_STATE_PREPARING {
                    clear_retention_staged_file_path(pool, &prepared_key, &staged_file_path, true)
                        .await?;
                    // The replacement either committed before the journal transition or never
                    // started. Re-run the normal writer from the durable manifest rather than
                    // treating the current artifact as a completed prepared publication.
                    continue;
                }
                clear_retention_staged_file_path(pool, &prepared_key, &staged_file_path, false)
                    .await?;
            } else if staged_path.is_file() {
                let Some(expected_sha) = manifest_sha.as_deref() else {
                    let error = anyhow!(
                        "prepared archive rollback cannot be verified without a manifest digest"
                    );
                    retention_recovery_persist_failure(
                        pool,
                        &prepared_key,
                        "legacy_reconcile",
                        &error,
                    )
                    .await?;
                    continue;
                };
                if let Err(error) =
                    restore_staged_legacy_archive_file(staged_path, path, expected_sha)
                {
                    retention_recovery_persist_failure(
                        pool,
                        &prepared_key,
                        "legacy_reconcile",
                        &error,
                    )
                    .await?;
                    continue;
                }
                clear_retention_staged_file_path(pool, &prepared_key, &staged_file_path, true)
                    .await?;
                // The manifest still describes the previous artifact. Leave the prepared row in
                // preparing state so the normal archive writer retries from the restored file.
                continue;
            }
        }
        if !retention_archive_path_is_owned(config, path)
            && state != RETENTION_RECOVERY_STATE_QUARANTINED
        {
            if state == RETENTION_RECOVERY_STATE_PREPARING
                || state == RETENTION_RECOVERY_STATE_PUBLISHED
            {
                let error = anyhow!("prepared archive path failed ownership verification");
                retention_recovery_persist_failure(pool, &prepared_key, "legacy_reconcile", &error)
                    .await?;
            }
            continue;
        }
        let _archive_lock = retention_archive_file_lock(path)?;
        if state == RETENTION_RECOVERY_STATE_PREPARING && path.is_file() {
            let actual_sha = sha256_hex_file(path)?;
            let descriptor = RetentionPreparedArchiveDescriptor {
                prepared_key: prepared_key.clone(),
                dataset: "codex_invocations",
                month_key,
                day_key,
                part_key,
                file_path: file_path.clone(),
                source_ids_json,
                source_identity_sha256,
            };
            if let Err(error) =
                verify_prepared_retention_archive_artifact(pool, &descriptor, path, &actual_sha)
                    .await
            {
                retention_recovery_persist_failure(pool, &prepared_key, "legacy_reconcile", &error)
                    .await?;
                continue;
            }
            let Some(admission) =
                acquire_retention_write_admission("retention_recovery_reconcile").await
            else {
                return Err(retention_write_deferred("retention_recovery_reconcile"));
            };
            let updated = sqlx::query(
                r#"
                UPDATE retention_prepared_archives
                SET state = ?1,
                    artifact_sha256 = ?2,
                    artifact_bytes = ?3,
                    updated_at = datetime('now')
                WHERE prepared_key = ?4
                  AND state = 'preparing'
                  AND staged_file_path IS NULL
                "#,
            )
            .bind(RETENTION_RECOVERY_STATE_PUBLISHED)
            .bind(actual_sha)
            .bind(fs::metadata(path)?.len() as i64)
            .bind(&prepared_key)
            .execute(pool)
            .await?;
            drop(admission);
            if updated.rows_affected() == 0 {
                continue;
            }
            continue;
        }
        if state == RETENTION_RECOVERY_STATE_PUBLISHED {
            let valid = path.is_file()
                && artifact_sha256.as_deref().is_some_and(|expected| {
                    sha256_hex_file(path).ok().as_deref() == Some(expected)
                });
            if !valid {
                let error = anyhow!("published retention archive artifact failed verification");
                retention_recovery_record_failure("legacy_reconcile", &error);
                let Some(admission) =
                    acquire_retention_write_admission("retention_recovery_quarantine").await
                else {
                    return Err(retention_write_deferred("retention_recovery_quarantine"));
                };
                quarantine_published_retention_archive_if_unchanged(
                    pool,
                    &prepared_key,
                    artifact_sha256.as_deref(),
                    retention_recovery_health_snapshot()
                        .failure_fingerprint
                        .as_deref(),
                )
                .await?;
                drop(admission);
            }
        } else if state == RETENTION_RECOVERY_STATE_QUARANTINED {
            let expired = quarantined_at.is_some()
                && sqlx::query_scalar::<_, i64>(
                    r#"
                    SELECT COALESCE(
                        julianday('now') - julianday(quarantined_at) >= (?1 / 86400.0),
                        0
                    )
                    FROM retention_prepared_archives
                    WHERE prepared_key = ?2
                    "#,
                )
                .bind(RETENTION_RECOVERY_QUARANTINE_GRACE_SECS)
                .bind(&prepared_key)
                .fetch_optional(pool)
                .await?
                .unwrap_or_default()
                    != 0;
            if expired {
                let _archive_lock = retention_archive_file_lock(path)?;
                let artifact_matches = artifact_sha256.as_deref().is_some_and(|expected| {
                    path.is_file() && sha256_hex_file(path).ok().as_deref() == Some(expected)
                });
                let Some(admission) =
                    acquire_retention_write_admission("retention_recovery_quarantine_cleanup")
                        .await
                else {
                    return Err(retention_write_deferred(
                        "retention_recovery_quarantine_cleanup",
                    ));
                };
                let mut tx = pool.begin().await?;
                let (manifest_exists, active_prepared_exists) = sqlx::query_as::<_, (i64, i64)>(
                    r#"
                        SELECT
                            EXISTS(SELECT 1 FROM archive_batches WHERE file_path = ?1),
                            EXISTS(
                                SELECT 1 FROM retention_prepared_archives
                                WHERE file_path = ?1
                                  AND prepared_key <> ?2
                                  AND state IN ('preparing', 'published')
                            )
                        "#,
                )
                .bind(&file_path)
                .bind(&prepared_key)
                .fetch_one(tx.as_mut())
                .await?;
                if manifest_exists == 0 && active_prepared_exists == 0 {
                    if artifact_matches && path.is_file() {
                        fs::remove_file(path).with_context(
                            || "failed to remove expired quarantined archive artifact",
                        )?;
                    }
                    sqlx::query("DELETE FROM retention_prepared_archives WHERE prepared_key = ?1")
                        .bind(prepared_key)
                        .execute(tx.as_mut())
                        .await?;
                }
                tx.commit().await?;
                drop(admission);
            }
        }
    }
    retention_recovery_refresh_counts(pool, config).await?;
    Ok(())
}

async fn reconcile_staged_archive_replacements(
    pool: &Pool<Sqlite>,
    config: Option<&AppConfig>,
) -> Result<()> {
    let rows = sqlx::query_as::<_, (i64, String)>(
        "SELECT id, file_path
         FROM archive_batches
         WHERE replacement_staged_path IS NOT NULL
         ORDER BY id ASC LIMIT ?1",
    )
    .bind(RETENTION_RECOVERY_LEGACY_SCAN_BATCH as i64)
    .fetch_all(pool)
    .await?;
    for (id, file_path) in rows {
        let path = Path::new(&file_path);
        let archive_lock = retention_archive_file_lock(path)?;
        if !archive_lock.is_held() {
            continue;
        }
        let Some((current_file_path, staged_file_path, manifest_sha)) =
            sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
                "SELECT file_path, replacement_staged_path, sha256
                 FROM archive_batches
                 WHERE id = ?1",
            )
            .bind(id)
            .fetch_optional(pool)
            .await?
        else {
            continue;
        };
        let Some(staged_file_path) = staged_file_path else {
            continue;
        };
        if current_file_path != file_path {
            continue;
        }
        let staged_path = Path::new(&staged_file_path);
        let Some(manifest_sha) = manifest_sha else {
            retention_recovery_record_failure(
                "legacy_reconcile",
                &anyhow!(
                    "archive replacement rollback cannot be verified without a manifest digest"
                ),
            );
            continue;
        };
        if config.is_some_and(|config| {
            !retention_archive_path_is_within_archive_root(config, path)
                || !retention_archive_path_is_within_archive_root(config, staged_path)
        }) || !replacement_staging_path_is_owned(path, staged_path)
        {
            retention_recovery_record_failure(
                "legacy_reconcile",
                &anyhow!("archive replacement staging path failed ownership verification"),
            );
            continue;
        }
        let current_sha = if path.is_file() {
            Some(sha256_hex_file(path)?)
        } else {
            None
        };
        if current_sha.as_deref() == Some(manifest_sha.as_str()) {
            if staged_path.is_file() {
                fs::remove_file(staged_path)
                    .context("failed to remove committed archive rollback copy")?;
            }
            clear_archive_batch_staged_path(pool, id, &staged_file_path).await?;
            continue;
        }
        if staged_path.is_file() {
            let staged_sha = sha256_hex_file(staged_path)?;
            if current_sha.as_deref() == Some(staged_sha.as_str()) && staged_sha != manifest_sha {
                let Some(_admission) =
                    acquire_retention_write_admission("retention_recovery_manifest_adopt").await
                else {
                    return Err(retention_write_deferred(
                        "retention_recovery_manifest_adopt",
                    ));
                };
                sqlx::query(
                    "UPDATE archive_batches
                     SET sha256 = ?1
                     WHERE id = ?2 AND replacement_staged_path = ?3",
                )
                .bind(&staged_sha)
                .bind(id)
                .bind(&staged_file_path)
                .execute(pool)
                .await?;
                match fs::remove_file(staged_path) {
                    Ok(()) => {
                        clear_archive_batch_staged_path(pool, id, &staged_file_path).await?;
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        clear_archive_batch_staged_path(pool, id, &staged_file_path).await?;
                    }
                    Err(error) => {
                        retention_recovery_record_failure("legacy_reconcile", &error.into());
                    }
                }
                continue;
            }
            if let Err(error) = restore_staged_legacy_archive_file(staged_path, path, &manifest_sha)
            {
                retention_recovery_record_failure("legacy_reconcile", &error);
                continue;
            }
            clear_archive_batch_staged_path(pool, id, &staged_file_path).await?;
        } else {
            retention_recovery_record_failure(
                "legacy_reconcile",
                &anyhow!("archive replacement staging artifact is missing"),
            );
        }
    }
    Ok(())
}

fn replacement_staging_path_is_owned(final_path: &Path, staged_path: &Path) -> bool {
    let Some(final_parent) = final_path.parent() else {
        return false;
    };
    let Some(staged_parent) = staged_path.parent() else {
        return false;
    };
    if final_parent != staged_parent {
        return false;
    }
    let Some(final_name) = final_path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some(staged_name) = staged_path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if !staged_name.starts_with(&format!("{final_name}.")) || !staged_name.ends_with(".restore") {
        return false;
    }
    fs::canonicalize(final_parent).is_ok_and(|parent| {
        !fs::symlink_metadata(final_path).is_ok_and(|metadata| metadata.file_type().is_symlink())
            && !fs::symlink_metadata(staged_path)
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
            && parent.is_dir()
    })
}

async fn clear_archive_batch_staged_path(
    pool: &Pool<Sqlite>,
    archive_batch_id: i64,
    expected_staged_file_path: &str,
) -> Result<()> {
    let Some(_admission) =
        acquire_retention_write_admission("retention_recovery_staged_path").await
    else {
        return Err(retention_write_deferred("retention_recovery_staged_path"));
    };
    sqlx::query(
        "UPDATE archive_batches SET replacement_staged_path = NULL
         WHERE id = ?1 AND replacement_staged_path = ?2",
    )
    .bind(archive_batch_id)
    .bind(expected_staged_file_path)
    .execute(pool)
    .await?;
    Ok(())
}

async fn clear_retention_staged_file_path(
    pool: &Pool<Sqlite>,
    prepared_key: &str,
    expected_staged_file_path: &str,
    reset_artifact: bool,
) -> Result<()> {
    let Some(_admission) =
        acquire_retention_write_admission("retention_recovery_staged_path").await
    else {
        return Err(retention_write_deferred("retention_recovery_staged_path"));
    };
    let query = if reset_artifact {
        "UPDATE retention_prepared_archives
         SET state = 'preparing', artifact_sha256 = NULL, artifact_bytes = NULL,
             staged_file_path = NULL, next_retry_at = NULL, updated_at = datetime('now')
         WHERE prepared_key = ?1 AND staged_file_path = ?2"
    } else {
        "UPDATE retention_prepared_archives
         SET staged_file_path = NULL, updated_at = datetime('now')
         WHERE prepared_key = ?1 AND staged_file_path = ?2"
    };
    sqlx::query(query)
        .bind(prepared_key)
        .bind(expected_staged_file_path)
        .execute(pool)
        .await?;
    Ok(())
}

fn restore_staged_legacy_archive_file(
    staged_path: &Path,
    final_path: &Path,
    expected_sha256: &str,
) -> Result<()> {
    let staged_sha256 = sha256_hex_file(staged_path)?;
    if staged_sha256 != expected_sha256 {
        bail!("archive rollback artifact digest verification failed");
    }
    match fs::rename(staged_path, final_path) {
        Ok(()) => sync_restored_archive_file(final_path),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            fs::remove_file(final_path).with_context(|| {
                format!(
                    "failed to replace interrupted legacy archive {}",
                    final_path.display()
                )
            })?;
            fs::rename(staged_path, final_path)
                .with_context(|| {
                    format!(
                        "failed to restore interrupted legacy archive {}",
                        final_path.display()
                    )
                })
                .and_then(|()| sync_restored_archive_file(final_path))
        }
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to restore interrupted legacy archive {}",
                final_path.display()
            )
        }),
    }
}

fn sync_restored_archive_file(final_path: &Path) -> Result<()> {
    fs::File::open(final_path)
        .context("failed to open restored archive for sync")?
        .sync_all()
        .context("failed to sync restored archive")?;
    if let Some(parent) = final_path.parent() {
        fs::File::open(parent)
            .context("failed to open restored archive directory for sync")?
            .sync_all()
            .context("failed to sync restored archive directory")?;
    }
    Ok(())
}

pub(crate) async fn reconcile_legacy_retention_archive_segments(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<()> {
    retention_recovery_set_stage("legacy_reconcile");

    // Probe pressure and write admission before touching the archive tree. The permit is
    // released immediately so archive verification does not hold the coordinator across file
    // I/O; each database mutation below still takes its own short admission.
    let Some(admission) =
        acquire_retention_write_admission("retention_recovery_legacy_quarantine").await
    else {
        return Err(retention_write_deferred(
            "retention_recovery_legacy_quarantine",
        ));
    };
    drop(admission);

    let archive_root = resolved_archive_dir(config).join("codex_invocations");
    let cursor = sqlx::query_scalar::<_, String>(
        "SELECT cursor FROM retention_recovery_cursors WHERE scope = 'legacy_archive_segments'",
    )
    .fetch_optional(pool)
    .await?
    .unwrap_or_default();
    let RetentionArchiveCandidateScan {
        candidates,
        progress,
    } = collect_retention_archive_candidates_after_cursor(
        &archive_root,
        cursor.as_str(),
        RETENTION_RECOVERY_LEGACY_SCAN_BATCH,
    )?;
    for path in &candidates {
        #[cfg(test)]
        retention_test_legacy_archive_io_event();
        let file_path = path.to_string_lossy().to_string();
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                retention_recovery_record_failure(
                    "legacy_reconcile",
                    &anyhow!("legacy archive metadata check failed: {}", error.kind()),
                );
                continue;
            }
        };
        let _archive_lock = retention_archive_file_lock(path)?;
        let sha256 = match sha256_hex_file(path) {
            Ok(sha256) => Some(sha256),
            Err(error) => {
                retention_recovery_record_failure("legacy_reconcile", &error);
                None
            }
        };
        let verified = if let Some(sha256) = sha256.as_deref() {
            match verify_legacy_retention_archive_segment(pool, config, path, sha256).await {
                Ok(verified) => verified,
                Err(error) => {
                    retention_recovery_record_failure("legacy_reconcile", &error);
                    None
                }
            }
        } else {
            None
        };
        let Some(admission) =
            acquire_retention_write_admission("retention_recovery_legacy_quarantine").await
        else {
            return Err(retention_write_deferred(
                "retention_recovery_legacy_quarantine",
            ));
        };
        let mut tx = pool.begin().await?;
        let referenced = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM archive_batches WHERE dataset = 'codex_invocations' AND file_path = ?1
            )
            OR EXISTS(
                SELECT 1 FROM retention_prepared_archives WHERE dataset = 'codex_invocations' AND file_path = ?1
            )
            "#,
        )
        .bind(&file_path)
        .fetch_one(tx.as_mut())
        .await?;
        if referenced != 0 {
            tx.rollback().await?;
            drop(admission);
            continue;
        }
        if let Some((descriptor, verified_sha)) = verified {
            let source_ids_json = descriptor.source_ids_json.clone();
            let (coverage_start_at, coverage_end_at): (Option<String>, Option<String>) =
                sqlx::query_as(
                    "SELECT MIN(occurred_at), MAX(occurred_at) FROM codex_invocations \
                     WHERE id IN (SELECT value FROM json_each(?1))",
                )
                .bind(&source_ids_json)
                .fetch_one(tx.as_mut())
                .await?;
            let archive_expires_at = coverage_end_at
                .as_deref()
                .map(|value| {
                    shanghai_archive_expiry_from_reference_timestamp(
                        value,
                        config.invocation_archive_ttl_days,
                    )
                })
                .transpose()?;
            let row_count = serde_json::from_str::<Vec<i64>>(&source_ids_json)?.len() as i64;
            sqlx::query(
                r#"
                INSERT INTO archive_batches (
                    dataset, month_key, day_key, part_key, file_path, sha256, row_count,
                    status, layout, codec, writer_version, cleanup_state,
                    coverage_start_at, coverage_end_at, archive_expires_at,
                    summary_source_kind, historical_rollups_materialized_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                        ?13, ?14, ?15, ?16, datetime('now'))
                "#,
            )
            .bind(descriptor.dataset)
            .bind(&descriptor.month_key)
            .bind(&descriptor.day_key)
            .bind(&descriptor.part_key)
            .bind(&descriptor.file_path)
            .bind(verified_sha)
            .bind(row_count)
            .bind(ARCHIVE_STATUS_COMPLETED)
            .bind(ARCHIVE_LAYOUT_SEGMENT_V1)
            .bind(ARCHIVE_FILE_CODEC_GZIP)
            .bind(ARCHIVE_WRITER_VERSION_SEGMENT_V1)
            .bind(ARCHIVE_CLEANUP_STATE_ACTIVE)
            .bind(coverage_start_at)
            .bind(coverage_end_at)
            .bind(archive_expires_at)
            .bind(SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR)
            .execute(tx.as_mut())
            .await?;
            sqlx::query("DELETE FROM retention_prepared_archives WHERE prepared_key = ?1")
                .bind(&descriptor.prepared_key)
                .execute(tx.as_mut())
                .await?;
        } else {
            sqlx::query(
                r#"
                INSERT OR IGNORE INTO retention_prepared_archives (
                    prepared_key, dataset, month_key, file_path, source_ids_json,
                    source_identity_sha256, state, artifact_sha256, artifact_bytes,
                    attempt_count, last_failure_stage, last_failure_fingerprint, quarantined_at
                )
                VALUES (?1, 'codex_invocations', '', ?2, '[]', 'legacy-unverified',
                        ?3, ?4, ?5, 0, 'legacy_reconcile', ?6, datetime('now'))
                "#,
            )
            .bind(format!("legacy:{file_path}"))
            .bind(&file_path)
            .bind(RETENTION_RECOVERY_STATE_QUARANTINED)
            .bind(sha256)
            .bind(metadata.len() as i64)
            .bind({
                let digest = Sha256::digest(b"legacy_archive_identity_unverified");
                format!("{digest:x}")[..16].to_string()
            })
            .execute(tx.as_mut())
            .await?;
        }
        tx.commit().await?;
        drop(admission);
    }
    let progress_after_cursor = progress.as_deref().filter(|path| *path > cursor.as_str());
    if let Some(progress) = progress_after_cursor {
        advance_retention_recovery_cursor(pool, cursor.as_str(), progress).await?;
    } else if !cursor.is_empty() {
        // Wrap when the tail is exhausted so earlier-arriving files are eventually revisited.
        let Some(admission) = acquire_retention_write_admission("retention_recovery_cursor").await
        else {
            return Err(retention_write_deferred("retention_recovery_cursor"));
        };
        sqlx::query(
            r#"
            UPDATE retention_recovery_cursors
            SET cursor = '', updated_at = datetime('now')
            WHERE scope = 'legacy_archive_segments' AND cursor = ?1
            "#,
        )
        .bind(&cursor)
        .execute(pool)
        .await?;
        drop(admission);
    }
    retention_recovery_refresh_counts(pool, config).await?;
    Ok(())
}

pub(crate) async fn advance_retention_recovery_cursor(
    pool: &Pool<Sqlite>,
    observed_cursor: &str,
    progress: &str,
) -> Result<()> {
    let Some(admission) = acquire_retention_write_admission("retention_recovery_cursor").await
    else {
        return Err(retention_write_deferred("retention_recovery_cursor"));
    };
    sqlx::query(
        r#"
        INSERT INTO retention_recovery_cursors (scope, cursor, updated_at)
        VALUES ('legacy_archive_segments', ?1, datetime('now'))
        ON CONFLICT(scope) DO UPDATE SET
            cursor = CASE
                WHEN retention_recovery_cursors.cursor = ?2
                    AND excluded.cursor > retention_recovery_cursors.cursor
                    THEN excluded.cursor
                ELSE retention_recovery_cursors.cursor
            END,
            updated_at = CASE
                WHEN retention_recovery_cursors.cursor = ?2
                    AND excluded.cursor > retention_recovery_cursors.cursor
                    THEN excluded.updated_at
                ELSE retention_recovery_cursors.updated_at
            END
        "#,
    )
    .bind(progress)
    .bind(observed_cursor)
    .execute(pool)
    .await?;
    drop(admission);
    Ok(())
}

struct RetentionArchiveCandidateScan {
    candidates: Vec<PathBuf>,
    progress: Option<String>,
}

impl RetentionArchiveCandidateScan {
    fn record_progress(&mut self, cursor_key: &str) {
        let should_replace = self
            .progress
            .as_ref()
            .is_none_or(|current| cursor_key > current.as_str());
        if should_replace {
            self.progress = Some(cursor_key.to_string());
        }
    }
}

fn collect_retention_archive_candidates_after_cursor(
    root: &Path,
    cursor: &str,
    limit: usize,
) -> Result<RetentionArchiveCandidateScan> {
    let mut scan = RetentionArchiveCandidateScan {
        candidates: Vec::with_capacity(limit.min(RETENTION_RECOVERY_LEGACY_SCAN_BATCH)),
        progress: None,
    };
    collect_retention_archive_candidates_after_cursor_inner(root, cursor, limit, &mut scan)?;
    // Filesystem enumeration order is not a cursor contract. Only sort the bounded candidate
    // set, never the complete contents of a visited directory.
    scan.candidates.sort_unstable();
    Ok(scan)
}

fn collect_retention_archive_candidates_after_cursor_inner(
    root: &Path,
    cursor: &str,
    limit: usize,
    scan: &mut RetentionArchiveCandidateScan,
) -> Result<bool> {
    if scan.candidates.len() >= limit {
        return Ok(true);
    }
    #[cfg(test)]
    retention_test_legacy_directory_traversal_event();
    if limit == 0 || !root.is_dir() {
        return Ok(false);
    }

    let mut directory_cursor = cursor.to_string();
    let mut skip_path: Option<PathBuf> = None;
    let mut pause_at: Option<String> = None;
    loop {
        let selection = collect_bounded_retention_archive_directory_entries(
            root,
            &directory_cursor,
            skip_path.as_deref(),
            limit,
        )?;
        let RetentionArchiveDirectorySelection { entries, truncated } = selection;
        if pause_at.is_none() && truncated {
            pause_at = entries.last().map(|entry| entry.cursor_key.clone());
        }
        let entries = entries.into_iter();
        let mut next_directory = None;
        for entry in entries {
            if entry.is_dir {
                next_directory = Some(entry);
                break;
            }
            let path = entry.path;
            let cursor_key = entry.cursor_key;
            let is_after_cursor = cursor_key.as_str() > directory_cursor.as_str();
            if is_after_cursor {
                scan.record_progress(&cursor_key);
                directory_cursor = cursor_key;
            }
            if entry.is_file
                && is_after_cursor
                && (path.to_string_lossy().ends_with(".sqlite.gz")
                    || path.to_string_lossy().ends_with(".sqlite.zst"))
            {
                scan.candidates.push(path);
            }
            if scan.candidates.len() >= limit {
                break;
            }
        }

        if scan.candidates.len() >= limit {
            return Ok(true);
        }
        let Some(entry) = next_directory else {
            if pause_at
                .as_deref()
                .is_some_and(|boundary| directory_cursor.as_str() >= boundary)
            {
                return Ok(true);
            }
            return Ok(truncated);
        };
        let RetentionArchiveDirectoryEntry {
            path,
            cursor_key,
            #[cfg(test)]
            heap_entry_guard,
            ..
        } = entry;
        #[cfg(test)]
        drop(heap_entry_guard);
        if cursor_key.as_str() > directory_cursor.as_str() {
            scan.record_progress(&cursor_key);
            directory_cursor = cursor_key;
        }
        let child_stopped = collect_retention_archive_candidates_after_cursor_inner(
            &path,
            &directory_cursor,
            limit,
            scan,
        )?;
        if child_stopped {
            return Ok(true);
        }
        if let Some(progress) = scan.progress.as_deref()
            && progress > directory_cursor.as_str()
        {
            directory_cursor = progress.to_string();
        }
        skip_path = Some(path);
        if truncated
            && pause_at
                .as_deref()
                .is_some_and(|boundary| directory_cursor.as_str() >= boundary)
        {
            return Ok(true);
        }
    }
}

fn retention_archive_cursor_key(path: &Path, is_dir: bool) -> String {
    let mut cursor_key = path.to_string_lossy().to_string();
    if is_dir {
        cursor_key.push('/');
    }
    cursor_key
}

#[derive(Debug, Eq, PartialEq)]
struct RetentionArchiveDirectoryEntry {
    path: PathBuf,
    cursor_key: String,
    is_dir: bool,
    is_file: bool,
    #[cfg(test)]
    heap_entry_guard: Option<RetentionTestLegacyDirectoryHeapEntryGuard>,
}

struct RetentionArchiveDirectorySelection {
    entries: Vec<RetentionArchiveDirectoryEntry>,
    truncated: bool,
}

impl Ord for RetentionArchiveDirectoryEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.cursor_key.cmp(&other.cursor_key)
    }
}

impl PartialOrd for RetentionArchiveDirectoryEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

fn collect_bounded_retention_archive_directory_entries(
    root: &Path,
    cursor: &str,
    skip_path: Option<&Path>,
    limit: usize,
) -> Result<RetentionArchiveDirectorySelection> {
    let directory_entries = fs::read_dir(root)
        .with_context(|| format!("failed to read archive directory {}", root.display()))?
        .map(|entry| {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            let is_dir = file_type.is_dir();
            Ok(RetentionArchiveDirectoryEntry {
                cursor_key: retention_archive_cursor_key(&path, is_dir),
                path,
                is_dir,
                is_file: file_type.is_file(),
                #[cfg(test)]
                heap_entry_guard: None,
            })
        });
    select_bounded_retention_archive_directory_entries(directory_entries, cursor, skip_path, limit)
}

fn select_bounded_retention_archive_directory_entries(
    directory_entries: impl IntoIterator<Item = Result<RetentionArchiveDirectoryEntry>>,
    cursor: &str,
    skip_path: Option<&Path>,
    limit: usize,
) -> Result<RetentionArchiveDirectorySelection> {
    // Stream the directory through a fixed-size max heap. This keeps allocation and comparison
    // work per entry bounded by the scan batch without materializing or sorting the full listing.
    let mut entries = std::collections::BinaryHeap::with_capacity(limit);
    let mut truncated = false;
    for entry in directory_entries {
        let entry = entry?;
        #[cfg(test)]
        retention_test_legacy_directory_entry_event();
        let path = &entry.path;
        let eligible = if entry.is_dir {
            skip_path.is_none_or(|skipped| skipped != path)
                && (cursor.is_empty()
                    || entry.cursor_key.as_str() > cursor
                    || Path::new(cursor).starts_with(path))
        } else {
            entry.cursor_key.as_str() > cursor
        };
        if !eligible {
            continue;
        }
        if entries.len() < limit {
            #[cfg(test)]
            let candidate = {
                let mut candidate = entry;
                candidate.heap_entry_guard =
                    Some(RetentionTestLegacyDirectoryHeapEntryGuard::new());
                candidate
            };
            #[cfg(not(test))]
            let candidate = entry;
            entries.push(candidate);
        } else if entries
            .peek()
            .is_some_and(|largest| entry.cursor_key < largest.cursor_key)
        {
            truncated = true;
            entries.pop();
            #[cfg(test)]
            let candidate = {
                let mut candidate = entry;
                candidate.heap_entry_guard =
                    Some(RetentionTestLegacyDirectoryHeapEntryGuard::new());
                candidate
            };
            #[cfg(not(test))]
            let candidate = entry;
            entries.push(candidate);
        } else {
            truncated = true;
        }
    }
    let mut entries = entries.into_vec();
    entries.sort_unstable_by(|left, right| left.cursor_key.cmp(&right.cursor_key));
    Ok(RetentionArchiveDirectorySelection { entries, truncated })
}

#[cfg(test)]
pub(crate) fn retention_test_select_bounded_archive_paths(
    paths: Vec<PathBuf>,
    cursor: &str,
    limit: usize,
) -> Vec<PathBuf> {
    let entries = paths.into_iter().map(|path| {
        Ok(RetentionArchiveDirectoryEntry {
            cursor_key: retention_archive_cursor_key(&path, false),
            path,
            is_dir: false,
            is_file: true,
            heap_entry_guard: None,
        })
    });
    select_bounded_retention_archive_directory_entries(entries, cursor, None, limit)
        .expect("select bounded archive paths")
        .entries
        .into_iter()
        .map(|entry| entry.path)
        .collect()
}

async fn verify_legacy_retention_archive_segment(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    archive_path: &Path,
    expected_sha256: &str,
) -> Result<Option<(RetentionPreparedArchiveDescriptor, String)>> {
    let file_name = archive_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let Some(part_key) = file_name.strip_suffix(".sqlite.gz") else {
        return Ok(None);
    };
    if !part_key.starts_with("part-") {
        return Ok(None);
    }
    let Some(day) = archive_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
    else {
        return Ok(None);
    };
    let Some(month) = archive_path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
    else {
        return Ok(None);
    };
    let Some(year) = archive_path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
    else {
        return Ok(None);
    };
    let day_key = format!("{year}-{month}-{day}");
    let temp_path = PathBuf::from(format!(
        "{}.{}.recovery.sqlite",
        archive_path.display(),
        retention_temp_suffix()
    ));
    let _temp_cleanup = TempSqliteCleanup(temp_path.clone());
    inflate_gzip_sqlite_file(archive_path, &temp_path)?;
    let mut archive_db = open_archive_sqlite_connection(&temp_path).await?;
    ensure_codex_invocations_archive_schema_direct(&mut archive_db).await?;
    let ids = sqlx::query_scalar::<_, i64>("SELECT id FROM codex_invocations ORDER BY id ASC")
        .fetch_all(&mut archive_db)
        .await?;
    if ids.is_empty() || archive_segment_part_key_for_ids(&ids).ok().as_deref() != Some(part_key) {
        return Ok(None);
    }
    let archive_identity = invocation_archive_source_identity_sha256(
        &mut archive_db,
        InvocationArchiveIdentityDatabase::Main,
        &ids,
    )
    .await?;
    archive_db.close().await?;
    let source_ids_json = serde_json::to_string(&ids)?;
    let mut source_connection = pool.acquire().await?;
    let source_identity = invocation_archive_source_identity_sha256(
        &mut source_connection,
        InvocationArchiveIdentityDatabase::Main,
        &ids,
    )
    .await?;
    drop(source_connection);
    if source_identity != archive_identity {
        return Ok(None);
    }
    let candidates = sqlx::query_as::<_, InvocationArchiveCandidate>(
        r#"
        SELECT id, invoke_id, occurred_at, source, status, input_tokens, output_tokens,
               cache_input_tokens, reasoning_tokens, total_tokens, cost, first_token_ms,
               payload, request_raw_path, response_raw_path
        FROM codex_invocations
        WHERE id IN (SELECT value FROM json_each(?1))
        ORDER BY id ASC
        "#,
    )
    .bind(&source_ids_json)
    .fetch_all(pool)
    .await?;
    if candidates.len() != ids.len() {
        return Ok(None);
    }
    let descriptor = retention_prepared_archive_descriptor(
        config,
        "codex_invocations",
        &day_key,
        &candidates,
        source_identity,
    )?;
    if descriptor.file_path != archive_path.to_string_lossy()
        || descriptor.part_key.as_deref() != Some(part_key)
    {
        return Ok(None);
    }
    if sha256_hex_file(archive_path)? != expected_sha256 {
        return Ok(None);
    }
    Ok(Some((descriptor, expected_sha256.to_string())))
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
        reasoning_tokens: candidate.reasoning_tokens,
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

pub(crate) fn encode_summary_archive_snapshot_v2_payload(
    rows: &[InvocationHourlySourceRecord],
    invoke_ids_by_row_id: &HashMap<i64, String>,
) -> Result<Vec<u8>> {
    let records = rows
        .iter()
        .map(|row| {
            let payload = row
                .payload
                .as_deref()
                .and_then(|payload| serde_json::from_str::<serde_json::Value>(payload).ok());
            SummaryArchiveSnapshotV2Record {
                id: row.id,
                // Preserve the durable invocation identity so a Snapshot fallback can deduplicate
                // against a live row that has not yet crossed the archive cleanup boundary.
                invoke_id: invoke_ids_by_row_id
                    .get(&row.id)
                    .cloned()
                    .or_else(|| {
                        payload
                            .as_ref()
                            .and_then(|value| value.get("invokeId"))
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| format!("archive-row-{}", row.id)),
                occurred_at: row.occurred_at.clone(),
                source: row.source.clone(),
                model: row.model.clone().or_else(|| {
                    payload
                        .as_ref()
                        .and_then(|value| value.get("model"))
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                }),
                response_model: payload
                    .as_ref()
                    .and_then(|value| value.get("responseModel"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                input_tokens: row.input_tokens.unwrap_or_default(),
                output_tokens: row.output_tokens.unwrap_or_default(),
                cache_input_tokens: row.cache_input_tokens.unwrap_or_default(),
                reasoning_tokens: row.reasoning_tokens.unwrap_or_default(),
                reasoning_effort: payload
                    .as_ref()
                    .and_then(|value| value.get("reasoningEffort"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                total_tokens: row.total_tokens.unwrap_or_default(),
                cost: row.cost,
                cost_input: row.cost_input,
                cost_cache_write: row.cost_cache_write,
                cost_cache_read: row.cost_cache_read,
                cost_output: row.cost_output,
                cost_reasoning: row.cost_reasoning,
                status: row.status.clone().unwrap_or_else(|| "unknown".to_string()),
                error_message: row.error_message.clone(),
                failure_kind: row.failure_kind.clone(),
                failure_class: row.failure_class.clone(),
                is_actionable: row.is_actionable.unwrap_or_default() != 0,
                upstream_account_id: row.resolved_upstream_account_id(),
            }
        })
        .collect::<Vec<_>>();
    let normalized =
        serde_json::to_vec(&records).context("encode normalized Summary Archive Snapshot page")?;
    zstd::stream::encode_all(normalized.as_slice(), 1)
        .context("compress normalized Summary Archive Snapshot page")
}

#[cfg(test)]
mod ttft_retention_tests {
    use super::*;

    #[test]
    fn archived_invocation_keeps_ttft_for_rollup_materialization() {
        let candidate = InvocationArchiveCandidate {
            id: 7,
            invoke_id: "invoke-7".to_string(),
            occurred_at: "2026-07-25 12:00:00".to_string(),
            source: SOURCE_PROXY.to_string(),
            status: Some("success".to_string()),
            input_tokens: Some(10),
            output_tokens: Some(2),
            cache_input_tokens: Some(3),
            reasoning_tokens: None,
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
    pub(crate) sha256: Option<String>,
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
    match run_data_retention_maintenance_with_circuit(
        &state.pool,
        &state.config,
        None,
        Some(cancel),
        state.raw_capture_circuit.clone(),
    )
    .await
    {
        Ok(summary) => {
            if summary.deferred {
                debug!(
                    trigger,
                    "retention maintenance deferred; preserving the prompt retry schedule"
                );
                invalidate_system_status_cache(state.as_ref()).await;
                return false;
            }
            // Commit the bounded inventory reset before task bookkeeping or cancellation can
            // return. Raw path mutations must never leave the monotonic inventory stale.
            let reset_pending = match crate::system_raw_payload_metrics_inventory_reset_pending(
                &state.pool,
            )
            .await
            {
                Ok(pending) => pending,
                Err(error) => {
                    warn!(
                        trigger,
                        error = %error,
                        "failed to inspect system raw metrics inventory reset state"
                    );
                    invalidate_system_status_cache(state.as_ref()).await;
                    return false;
                }
            };
            if summary.raw_files_compressed > 0
                || summary.raw_files_removed > 0
                || summary.orphan_raw_files_removed > 0
                || reset_pending
            {
                match reset_retention_raw_payload_metrics_inventory(state.as_ref()).await {
                    Ok(true) => {}
                    Ok(false) => {
                        debug!(
                            trigger,
                            "system raw metrics inventory reset deferred; preserving retry schedule"
                        );
                        invalidate_system_status_cache(state.as_ref()).await;
                        return false;
                    }
                    Err(error) => {
                        warn!(error = %error, "failed to reset system raw metrics inventory after retention");
                        invalidate_system_status_cache(state.as_ref()).await;
                        return false;
                    }
                }
            }
            let touched_anything = summary.touched_anything();
            if touched_anything && !summary.dry_run {
                let task_run = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return false,
                    result = begin_system_task_run_admitted(
                        state.as_ref(),
                        crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::MaintenanceRetention,
                        SystemTaskKind::RetentionArchive,
                        trigger,
                        Some("retention maintenance completed a write pass".to_string()),
                    ) => result.ok(),
                };
                if let Some(handle) = task_run.as_ref() {
                    let (brief, detail) = summarize_retention_run_for_system_task(&summary);
                    let _ = finish_system_task_run_reliably(
                        state.as_ref(),
                        Some(cancel),
                        handle,
                        SystemTaskStatus::Success,
                        Some(brief),
                        Some(detail),
                    )
                    .await;
                }
            }
            invalidate_system_status_cache(state.as_ref()).await;
            touched_anything
        }
        Err(err) => {
            let pressure_error = crate::db_pressure::global_db_pressure_gate()
                .record_error("data_retention_maintenance", &err);
            retention_record_error("data_retention_maintenance", &err);
            if !state.config.retention_dry_run {
                let task_run = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return false,
                    result = begin_system_task_run_admitted(
                        state.as_ref(),
                        crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::MaintenanceRetention,
                        SystemTaskKind::RetentionArchive,
                        trigger,
                        Some("retention maintenance failed".to_string()),
                    ) => result.ok(),
                };
                if let Some(handle) = task_run.as_ref() {
                    let _ = finish_system_task_run_reliably(
                        state.as_ref(),
                        Some(cancel),
                        handle,
                        SystemTaskStatus::Failed,
                        Some("retention maintenance failed".to_string()),
                        Some(format!(
                            "failure_fingerprint:{}",
                            retention_error_fingerprint(&err)
                        )),
                    )
                    .await;
                }
            }
            warn!(
                trigger,
                error_fingerprint = %retention_error_fingerprint(&err),
                retry_soon = pressure_error,
                "failed to run retention maintenance"
            );
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
    let defer_generation = retention_defer_generation();
    let run = async {
        if let Some(shutdown) = shutdown {
            RETENTION_SHUTDOWN
                .scope(
                    shutdown.clone(),
                    run_data_retention_maintenance_with_task_run_prune(
                        pool,
                        config,
                        dry_run_override,
                        Some(shutdown),
                    ),
                )
                .await
        } else {
            run_data_retention_maintenance_with_task_run_prune(pool, config, dry_run_override, None)
                .await
        }
    };
    let result = RETENTION_CURRENT_PREPARED_KEY
        .scope(RefCell::new(None), run)
        .await;
    result.map(|mut summary| {
        summary.deferred = retention_defer_generation() != defer_generation;
        summary
    })
}

async fn run_data_retention_maintenance_with_task_run_prune(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run_override: Option<bool>,
    shutdown: Option<&CancellationToken>,
) -> Result<RetentionRunSummary> {
    let dry_run = dry_run_override.unwrap_or(config.retention_dry_run);
    let result =
        run_data_retention_maintenance_inner(pool, config, dry_run_override, shutdown).await;
    // This janitor must run even when an earlier archive stage fails. Otherwise every
    // failed pass adds a task-run record while the retention policy that bounds those
    // records is unreachable until the unrelated failure clears.
    let task_run_prune = if should_stop_data_retention_maintenance(shutdown) {
        Ok(0)
    } else {
        prune_system_task_runs(pool, dry_run).await
    };
    match result {
        Ok(mut summary) => {
            summary.system_task_run_rows_pruned +=
                task_run_prune.context("failed to prune expired system task runs")?;
            Ok(summary)
        }
        Err(error) => {
            if let Err(prune_error) = task_run_prune {
                warn!(error = %prune_error, "failed to prune system task runs after retention failure");
            }
            Err(error)
        }
    }
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
        if let Err(error) = reconcile_staged_archive_replacements(pool, Some(config)).await {
            retention_recovery_persist_latest_failure_best_effort(pool, "legacy_reconcile", &error)
                .await;
            retention_recovery_log_event(
                tracing::Level::WARN,
                "staged_replacement_reconcile",
                "staged archive replacement reconciliation deferred",
            );
        }
        if let Err(error) = reconcile_retention_prepared_archives(pool, config).await {
            retention_recovery_persist_latest_failure_best_effort(pool, "legacy_reconcile", &error)
                .await;
            retention_recovery_log_event(
                tracing::Level::WARN,
                "prepared_reconcile",
                "retention prepared archive reconciliation deferred",
            );
        }
        if let Err(error) = reconcile_legacy_retention_archive_segments(pool, config).await {
            retention_recovery_persist_latest_failure_best_effort(pool, "legacy_reconcile", &error)
                .await;
            retention_recovery_log_event(
                tracing::Level::WARN,
                "legacy_reconcile",
                "legacy retention archive reconciliation deferred",
            );
        }
    }

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    retention_recovery_set_stage("orphan_sweep");
    match sweep_orphan_proxy_raw_files(pool, config, raw_path_fallback_root, dry_run).await {
        Ok(removed) => summary.orphan_raw_files_removed += removed,
        Err(error) => {
            if dry_run {
                retention_recovery_record_failure("orphan_sweep", &error);
            } else {
                retention_recovery_persist_latest_failure_best_effort(pool, "orphan_sweep", &error)
                    .await;
            }
            retention_recovery_log_event(
                tracing::Level::WARN,
                "orphan_sweep",
                "raw orphan sweep deferred; continuing independent retention stages",
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
        match prune_old_invocation_details(pool, config, raw_path_fallback_root, dry_run).await {
            Ok(pruned) => pruned,
            Err(error) => {
                if dry_run {
                    retention_recovery_record_failure("detail_prune", &error);
                } else if is_retention_write_deferred(&error) {
                    retention_recovery_record_deferred("detail_prune");
                } else if is_retention_recovery_failure_persisted(&error) {
                    retention_recovery_record_failure("detail_prune", &error);
                } else {
                    retention_recovery_persist_latest_failure_best_effort(
                        pool,
                        "detail_prune",
                        &error,
                    )
                    .await;
                }
                retention_recovery_log_event(
                    tracing::Level::WARN,
                    "detail_prune",
                    "invocation detail pruning failed; continuing independent retention stages",
                );
                (0, 0, 0)
            }
        }
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

    retention_recovery_clear_current_prepared_key();
    let invocation_archive = if invocation_payload_retention_ready {
        match archive_old_invocations(pool, config, raw_path_fallback_root, dry_run).await {
            Ok(archive) => archive,
            Err(error) => {
                if is_retention_write_deferred(&error) {
                    retention_recovery_record_deferred("finalizing");
                } else if is_retention_recovery_failure_persisted(&error) {
                    retention_recovery_record_failure("finalizing", &error);
                } else {
                    retention_recovery_persist_latest_failure_best_effort(
                        pool,
                        "finalizing",
                        &error,
                    )
                    .await;
                }
                retention_recovery_log_event(
                    tracing::Level::WARN,
                    "invocation_archive",
                    "invocation archive stage failed; continuing independent retention stages",
                );
                (0, 0, 0)
            }
        }
    } else {
        info!(
            payload_loss_days,
            "invocation archival deferred until parallel-work minute coverage catches up"
        );
        (0, 0, 0)
    };
    retention_recovery_clear_current_prepared_key();
    summary.invocation_rows_archived += invocation_archive.0;
    summary.archive_batches_touched += invocation_archive.1;
    summary.raw_files_removed += invocation_archive.2;
    if !dry_run {
        retention_recovery_set_stage("status_refresh");
        if let Err(error) = retention_recovery_refresh_counts(pool, config).await {
            retention_recovery_record_failure("status_refresh", &error);
            retention_recovery_log_event(
                tracing::Level::WARN,
                "status_refresh",
                "retention recovery status refresh failed; continuing independent retention stages",
            );
        } else {
            retention_recovery_log_event(
                tracing::Level::INFO,
                "status_refresh",
                "retention recovery status refreshed",
            );
        }
    }
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
                StartupBackfillTask::LegacyDetailMirrors,
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
    summary.raw_files_removed += proxy_archive.2;

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
    summary.raw_files_removed += pool_attempt_archive.2;

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
            if !dry_run && !prepare_raw_compression_inventory_reset(pool).await? {
                continue;
            }
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

async fn prepare_raw_compression_inventory_reset(pool: &Pool<Sqlite>) -> Result<bool> {
    let Some(admission) = acquire_retention_write_admission("raw_metrics_inventory_reset").await
    else {
        return Ok(false);
    };
    mark_retention_raw_inventory_reset_intent(pool).await?;
    drop(admission);
    Ok(true)
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

            if !dry_run && !prepare_raw_compression_inventory_reset(pool).await? {
                continue;
            }

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

async fn reset_retention_raw_payload_metrics_inventory(state: &AppState) -> Result<bool> {
    loop {
        let Some(complete) = reset_retention_raw_payload_metrics_inventory_batch(state).await?
        else {
            return Ok(false);
        };
        if complete {
            return Ok(true);
        }
    }
}

async fn reset_retention_raw_payload_metrics_inventory_batch(
    state: &AppState,
) -> Result<Option<bool>> {
    let Some(admission) = acquire_retention_write_admission("raw_metrics_inventory_reset").await
    else {
        return Ok(None);
    };
    let candidate_limit = retention_candidate_limit(&state.config, "raw_metrics_inventory_reset");
    let execute_started = Instant::now();
    let outcome = reset_system_raw_payload_metrics_inventory_batch(state, candidate_limit).await?;
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
    Ok(Some(outcome.complete))
}

pub(crate) async fn resume_retention_raw_payload_metrics_inventory_reset(
    state: &AppState,
) -> Result<bool> {
    if !crate::system_raw_payload_metrics_inventory_reset_pending(&state.pool).await? {
        return Ok(true);
    }
    reset_retention_raw_payload_metrics_inventory_batch(state)
        .await
        .map(|outcome| outcome.unwrap_or(false))
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
            let mut source_connection = pool.acquire().await?;
            let source_identity_sha256 = invocation_archive_source_identity_sha256(
                &mut source_connection,
                InvocationArchiveIdentityDatabase::Main,
                &ids,
            )
            .await?;
            drop(source_connection);
            let mut descriptor = retention_prepared_archive_descriptor(
                config,
                spec.dataset,
                &group_key,
                &load_invocation_archive_candidates_by_ids(pool, &ids).await?,
                source_identity_sha256,
            )?;
            descriptor.file_path = retention_live_mirror_archive_path(
                config,
                &group_key,
                &ids,
                &descriptor.source_identity_sha256,
            )?
            .to_string_lossy()
            .to_string();
            descriptor.prepared_key = format!(
                "{}:{}:{}",
                descriptor.dataset, descriptor.file_path, descriptor.source_identity_sha256
            );
            retention_recovery_set_current_prepared_key(&descriptor.prepared_key);
            if let Err(error) = retention_recovery_record_preparing(pool, &descriptor).await {
                if is_retention_write_deferred(&error) {
                    retention_recovery_record_deferred("preparing");
                    return Ok((rows_pruned, archive_batches, raw_files_removed));
                }
                if is_retention_recovery_failure_persisted(&error) {
                    retention_recovery_record_failure("preparing", &error);
                    return Err(error);
                }
                retention_recovery_persist_failure(
                    pool,
                    &descriptor.prepared_key,
                    "preparing",
                    &error,
                )
                .await?;
                return Err(retention_recovery_failure_persisted(
                    &descriptor.prepared_key,
                    error,
                ));
            }
            let archive_result = match archive_layout_for_dataset(config, spec.dataset) {
                ArchiveBatchLayout::LegacyMonth => {
                    archive_rows_into_month_batch_at_path(
                        pool,
                        spec,
                        &group_key,
                        &ids,
                        PathBuf::from(&descriptor.file_path),
                    )
                    .await
                }
                ArchiveBatchLayout::SegmentV1 => {
                    archive_rows_into_segment_batch_at_path(
                        pool,
                        config,
                        spec,
                        &group_key,
                        &ids,
                        PathBuf::from(&descriptor.file_path),
                    )
                    .await
                }
            };
            let Some(mut archive_outcome) = retention_prepared_batch_or_deferred(archive_result)?
            else {
                return Ok((rows_pruned, archive_batches, raw_files_removed));
            };
            if archive_outcome.source_identity_sha256.as_deref()
                != Some(descriptor.source_identity_sha256.as_str())
            {
                let error =
                    anyhow!("retention prepared archive source identity verification failed");
                retention_recovery_persist_failure(
                    pool,
                    &descriptor.prepared_key,
                    "preparing",
                    &error,
                )
                .await?;
                return Err(retention_recovery_failure_persisted(
                    &descriptor.prepared_key,
                    error,
                ));
            }
            set_archive_batch_coverage_from_local_rows(
                &mut archive_outcome,
                group.iter().map(|candidate| candidate.occurred_at.as_str()),
                Some(config.invocation_archive_ttl_days),
            )?;
            archive_outcome.summary_source_kind = SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR;
            retention_recovery_mark_published(pool, &descriptor, &archive_outcome.sha256).await?;
            let pruned_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
            let prepare_elapsed = prepare_started.elapsed();
            let _archive_lock = retention_archive_file_lock(Path::new(&archive_outcome.file_path))?;
            let actual_archive_sha256 = sha256_hex_file(Path::new(&archive_outcome.file_path))?;
            if actual_archive_sha256 != archive_outcome.sha256 {
                let error =
                    anyhow!("retention prepared archive artifact digest verification failed");
                retention_recovery_persist_failure(
                    pool,
                    &descriptor.prepared_key,
                    "publishing",
                    &error,
                )
                .await?;
                return Err(retention_recovery_failure_persisted(
                    &descriptor.prepared_key,
                    error,
                ));
            }
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
            retention_recovery_verify_publication_tx(
                tx.as_mut(),
                &descriptor,
                &archive_outcome.sha256,
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
            retention_recovery_delete_tx(tx.as_mut(), &descriptor.prepared_key).await?;
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
            let raw_paths = filter_unreferenced_proxy_raw_paths(pool, &raw_paths).await?;
            if !raw_paths.is_empty() {
                mark_retention_raw_inventory_reset_intent(pool).await?;
            }
            drop(admission);
            rows_pruned += group.len();
            archive_batches += 1;
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
                invoke_id,
                occurred_at,
                source,
                status,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                reasoning_tokens,
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
                invoke_id,
                occurred_at,
                source,
                status,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                reasoning_tokens,
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
            let ids = group
                .iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            let mut source_connection = pool.acquire().await?;
            let source_identity_sha256 = invocation_archive_source_identity_sha256(
                &mut source_connection,
                InvocationArchiveIdentityDatabase::Main,
                &ids,
            )
            .await?;
            drop(source_connection);
            let prepare_started = Instant::now();
            let descriptor = retention_prepared_archive_descriptor(
                config,
                spec.dataset,
                &group_key,
                &group,
                source_identity_sha256,
            )?;
            retention_recovery_set_current_prepared_key(&descriptor.prepared_key);
            if let Err(error) = retention_recovery_record_preparing(pool, &descriptor).await {
                if is_retention_write_deferred(&error) {
                    retention_recovery_record_deferred("preparing");
                    return Ok((rows_archived, archive_batches, raw_files_removed));
                }
                if is_retention_recovery_failure_persisted(&error) {
                    retention_recovery_record_failure("preparing", &error);
                    return Err(error);
                }
                retention_recovery_persist_failure(
                    pool,
                    &descriptor.prepared_key,
                    "preparing",
                    &error,
                )
                .await?;
                return Err(retention_recovery_failure_persisted(
                    &descriptor.prepared_key,
                    error,
                ));
            }
            let raw_paths = group
                .iter()
                .flat_map(|candidate| {
                    [
                        candidate.request_raw_path.clone(),
                        candidate.response_raw_path.clone(),
                    ]
                })
                .collect::<Vec<_>>();

            let materialized_rows = group
                .iter()
                .map(invocation_archive_candidate_to_hourly_source_record)
                .collect::<Vec<_>>();
            let archive_result = match archive_layout_for_dataset(config, spec.dataset) {
                ArchiveBatchLayout::LegacyMonth => {
                    archive_rows_into_month_batch(pool, config, spec, &group_key, &ids).await
                }
                ArchiveBatchLayout::SegmentV1 => {
                    archive_rows_into_segment_batch(pool, config, spec, &group_key, &ids).await
                }
            };
            let Some(mut archive_outcome) = (match archive_result {
                Ok(outcome) => Some(outcome),
                Err(error) if is_retention_write_deferred(&error) => {
                    retention_recovery_record_deferred("preparing");
                    return Ok((rows_archived, archive_batches, raw_files_removed));
                }
                Err(error) => {
                    retention_recovery_persist_failure(
                        pool,
                        &descriptor.prepared_key,
                        "preparing",
                        &error,
                    )
                    .await?;
                    return Err(retention_recovery_failure_persisted(
                        &descriptor.prepared_key,
                        error,
                    ));
                }
            }) else {
                return Ok((rows_archived, archive_batches, raw_files_removed));
            };
            if archive_outcome.source_identity_sha256.as_deref()
                != Some(descriptor.source_identity_sha256.as_str())
            {
                let error =
                    anyhow!("retention prepared archive source identity verification failed");
                retention_recovery_persist_failure(
                    pool,
                    &descriptor.prepared_key,
                    "preparing",
                    &error,
                )
                .await?;
                return Err(retention_recovery_failure_persisted(
                    &descriptor.prepared_key,
                    error,
                ));
            }
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
            archive_outcome.summary_source_kind = SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE;
            retention_recovery_mark_published(pool, &descriptor, &archive_outcome.sha256).await?;
            let prepare_elapsed = prepare_started.elapsed();
            let _archive_lock = retention_archive_file_lock(Path::new(&archive_outcome.file_path))?;
            let actual_archive_sha256 = sha256_hex_file(Path::new(&archive_outcome.file_path))?;
            if actual_archive_sha256 != archive_outcome.sha256 {
                let error =
                    anyhow!("retention prepared archive artifact changed before publication");
                retention_recovery_persist_failure(
                    pool,
                    &descriptor.prepared_key,
                    "publishing",
                    &error,
                )
                .await?;
                return Err(retention_recovery_failure_persisted(
                    &descriptor.prepared_key,
                    error,
                ));
            }
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
            stage_invocation_archive_batch_manifest(tx.as_mut(), &archive_outcome).await?;
            mark_archive_batch_historical_rollups_materialized_tx(
                tx.as_mut(),
                spec.dataset,
                &archive_outcome.file_path,
            )
            .await?;
            // Keep a normalized, raw-free Summary source page in the same transaction as the
            // authoritative archive publication. Cleanup can therefore retire the file only
            // after this exact page and its manifest identity are durable.
            let snapshot_archive_batch_id = load_archive_batch_id_for_file_tx(
                tx.as_mut(),
                spec.dataset,
                &archive_outcome.month_key,
                &archive_outcome.file_path,
            )
            .await?;
            let invoke_ids_by_row_id = group
                .iter()
                .map(|candidate| (candidate.id, candidate.invoke_id.clone()))
                .collect::<HashMap<_, _>>();
            let snapshot_payload = encode_summary_archive_snapshot_v2_payload(
                &materialized_rows,
                &invoke_ids_by_row_id,
            )?;
            let snapshot_page = SummaryArchiveSnapshotPage {
                archive_batch_id: snapshot_archive_batch_id,
                manifest_sha256: archive_outcome.sha256.clone(),
                page_index: 0,
                coverage_start: archive_outcome.coverage_start_at.clone().ok_or_else(|| {
                    anyhow!("authoritative archive is missing Snapshot start coverage")
                })?,
                coverage_end: archive_outcome.coverage_end_at.clone().ok_or_else(|| {
                    anyhow!("authoritative archive is missing Snapshot end coverage")
                })?,
                row_count: u32::try_from(materialized_rows.len())
                    .context("Summary Archive Snapshot row count overflow")?,
                payload: snapshot_payload,
            };
            store_summary_archive_snapshot_page_v2_tx(tx.as_mut(), &snapshot_page).await?;
            // The page is written as part of the authoritative archive transaction.  Mark the
            // V2 proof in that same transaction so cleanup can never observe a page without its
            // semantic authority.
            if !summary_archive_snapshot_has_proof_tx(
                tx.as_mut(),
                snapshot_archive_batch_id,
                &archive_outcome.sha256,
            )
            .await?
            {
                bail!("retention Summary Snapshot V2 semantic proof validation failed");
            }
            store_summary_archive_snapshot_v2_final_proof_tx(
                tx.as_mut(),
                snapshot_archive_batch_id,
                &archive_outcome.sha256,
            )
            .await?;
            retention_recovery_verify_publication_tx(
                tx.as_mut(),
                &descriptor,
                &archive_outcome.sha256,
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
            for target in INVOCATION_HOURLY_ROLLUP_TARGETS {
                mark_hourly_rollup_archive_replayed_tx(
                    tx.as_mut(),
                    target,
                    spec.dataset,
                    &archive_outcome.file_path,
                )
                .await?;
            }
            finalize_invocation_archive_batch_publication_tx(
                tx.as_mut(),
                &archive_outcome.file_path,
            )
            .await?;
            retention_recovery_delete_tx(tx.as_mut(), &descriptor.prepared_key).await?;
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
            let raw_paths = filter_unreferenced_proxy_raw_paths(pool, &raw_paths).await?;
            if !raw_paths.is_empty() {
                mark_retention_raw_inventory_reset_intent(pool).await?;
            }
            drop(admission);
            rows_archived += group.len();
            archive_batches += 1;
            retention_recovery_record_progress();
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
) -> Result<(usize, usize, usize)> {
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
            0,
        ));
    }

    let mut rows_archived = 0usize;
    let mut archive_batches = 0usize;
    let mut raw_files_removed = 0usize;

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
                return Ok((rows_archived, archive_batches, raw_files_removed));
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
            let _archive_lock = retention_archive_file_lock(Path::new(&archive_outcome.file_path))?;
            let actual_sha256 = match sha256_hex_file(Path::new(&archive_outcome.file_path)) {
                Ok(value) => value,
                Err(_) => return Ok((rows_archived, archive_batches, raw_files_removed)),
            };
            if actual_sha256 != archive_outcome.sha256 {
                return Ok((rows_archived, archive_batches, raw_files_removed));
            }
            let Some(admission) = acquire_retention_write_admission("timestamped_archive").await
            else {
                return Ok((rows_archived, archive_batches, raw_files_removed));
            };
            let execute_started = Instant::now();
            let mut tx = pool.begin().await?;
            let cleanup_state = sqlx::query_scalar::<_, Option<String>>(
                "SELECT cleanup_state FROM archive_batches WHERE dataset = ?1 AND month_key = ?2 AND file_path = ?3",
            )
            .bind(spec.dataset)
            .bind(&archive_outcome.month_key)
            .bind(&archive_outcome.file_path)
            .fetch_optional(tx.as_mut())
            .await?
            .flatten();
            if cleanup_state
                .as_deref()
                .is_some_and(|state| state != ARCHIVE_CLEANUP_STATE_ACTIVE)
            {
                tx.rollback().await?;
                drop(admission);
                return Ok((rows_archived, archive_batches, raw_files_removed));
            }
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
            if spec.dataset == "pool_upstream_request_attempts" {
                let raw_paths =
                    filter_unreferenced_proxy_raw_paths(pool, &pool_attempt_raw_paths).await?;
                if !raw_paths.is_empty() {
                    mark_retention_raw_inventory_reset_intent(pool).await?;
                }
                drop(admission);
                rows_archived += group.len();
                archive_batches += 1;
                raw_files_removed +=
                    delete_proxy_raw_paths(&raw_paths, config.database_path.parent())?;
            } else {
                drop(admission);
                rows_archived += group.len();
                archive_batches += 1;
            }
        }
    }

    Ok((rows_archived, archive_batches, raw_files_removed))
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
    fn retention_health_records_a_budget_breach_for_the_next_production_candidate() {
        const OPERATION: &str = "retention_test_adaptive_budget";
        let mut health = RetentionWriteHealthState::default();
        assert_eq!(
            retention_adaptive_candidate_limit_from_state(&mut health, 64, OPERATION),
            4
        );
        assert!(observe_retention_write_commit(
            &mut health,
            OPERATION,
            "normal",
            4,
            4 * 256,
            Duration::ZERO,
            Duration::ZERO,
            Duration::from_millis(251),
            Duration::ZERO,
            0,
            0,
        ));
        assert_eq!(
            retention_adaptive_candidate_limit_from_state(&mut health, 64, OPERATION),
            2
        );
        assert_eq!(health.snapshot.state, "degraded");
    }

    #[test]
    fn retention_micro_batch_keeps_a_single_oversized_row_losslessly() {
        let selected =
            take_retention_micro_batch(vec![2 * RETENTION_WRITE_MAX_BYTES, 128], |value| *value);
        assert_eq!(selected, vec![2 * RETENTION_WRITE_MAX_BYTES]);
    }

    #[test]
    fn system_task_run_retention_only_backs_off_for_pressure_cooldown() {
        assert!(
            system_task_run_retention_admission_requires_pressure_backoff(Some(
                crate::db_pressure::DbPressureDenyReason::PressureCooldown { remaining_ms: 1 }
            ))
        );
        assert!(
            !system_task_run_retention_admission_requires_pressure_backoff(Some(
                crate::db_pressure::DbPressureDenyReason::BackgroundBusy
            ))
        );
        assert!(!system_task_run_retention_admission_requires_pressure_backoff(None));
    }
}

#[cfg(test)]
mod retention_recovery_race_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn quarantine_guard_does_not_overwrite_a_republished_artifact() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect in-memory retention database");
        sqlx::query(
            r#"
            CREATE TABLE retention_prepared_archives (
                prepared_key TEXT PRIMARY KEY,
                state TEXT NOT NULL,
                artifact_sha256 TEXT,
                quarantined_at TEXT,
                last_failure_stage TEXT,
                last_failure_fingerprint TEXT,
                updated_at TEXT
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create prepared archive fixture");
        sqlx::query(
            "INSERT INTO retention_prepared_archives (prepared_key, state, artifact_sha256) VALUES ('race', 'published', 'old-sha')",
        )
        .execute(&pool)
        .await
        .expect("seed published archive fixture");

        sqlx::query(
            "UPDATE retention_prepared_archives SET state = 'published', artifact_sha256 = 'new-sha' WHERE prepared_key = 'race'",
        )
        .execute(&pool)
        .await
        .expect("republish archive fixture before stale quarantine update");

        let updated = quarantine_published_retention_archive_if_unchanged(
            &pool,
            "race",
            Some("old-sha"),
            Some("fingerprint"),
        )
        .await
        .expect("run stale quarantine guard");
        assert!(!updated);

        let row: (String, String) = sqlx::query_as(
            "SELECT state, artifact_sha256 FROM retention_prepared_archives WHERE prepared_key = 'race'",
        )
        .fetch_one(&pool)
        .await
        .expect("load republished archive fixture");
        assert_eq!(row, ("published".to_string(), "new-sha".to_string()));
    }

    #[test]
    fn staged_legacy_archive_restore_replaces_interrupted_artifact() {
        let root = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-staged-restore-{}-{}",
            std::process::id(),
            retention_temp_suffix()
        ));
        fs::create_dir_all(&root).expect("create staged restore directory");
        let final_path = root.join("archive.sqlite.gz");
        let staged_path = root.join("archive.sqlite.gz.restore");
        fs::write(&final_path, b"replacement artifact").expect("write replacement artifact");
        fs::write(&staged_path, b"previous artifact").expect("write previous artifact");
        let expected_sha = sha256_hex_file(&staged_path).expect("hash previous artifact");

        restore_staged_legacy_archive_file(&staged_path, &final_path, &expected_sha)
            .expect("restore staged legacy archive");
        assert_eq!(
            fs::read(&final_path).expect("read restored archive"),
            b"previous artifact"
        );
        assert!(!staged_path.exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn staged_archive_replacement_reconcile_restores_manifest_old_bytes() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect staged replacement database");
        sqlx::query(
            "CREATE TABLE archive_batches (
                id INTEGER PRIMARY KEY,
                file_path TEXT NOT NULL,
                replacement_staged_path TEXT,
                sha256 TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .expect("create staged replacement manifest");
        let root = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-staged-reconcile-{}-{}",
            std::process::id(),
            retention_temp_suffix()
        ));
        fs::create_dir_all(&root).expect("create staged reconcile directory");
        let final_path = root.join("archive.sqlite.gz");
        let staged_path = root.join("archive.sqlite.gz.restore");
        fs::write(&final_path, b"replacement artifact").expect("write replacement artifact");
        fs::write(&staged_path, b"previous artifact").expect("write previous artifact");
        let old_sha = sha256_hex_file(&staged_path).expect("hash previous artifact");
        let file_path = final_path.to_string_lossy().to_string();
        let staged_path_string = staged_path.to_string_lossy().to_string();
        sqlx::query(
            "INSERT INTO archive_batches (id, file_path, replacement_staged_path, sha256) VALUES (1, ?1, ?2, ?3)",
        )
        .bind(&file_path)
        .bind(&staged_path_string)
        .bind(&old_sha)
        .execute(&pool)
        .await
        .expect("seed staged replacement manifest");

        reconcile_staged_archive_replacements(&pool, None)
            .await
            .expect("reconcile staged replacement");
        assert_eq!(
            fs::read(&final_path).expect("read restored manifest file"),
            b"previous artifact"
        );
        let staged: Option<String> =
            sqlx::query_scalar("SELECT replacement_staged_path FROM archive_batches WHERE id = 1")
                .fetch_one(&pool)
                .await
                .expect("load cleared replacement staging path");
        assert!(staged.is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn staged_archive_replacement_reconcile_keeps_unverified_rollback_pending() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect unverified staged replacement database");
        sqlx::query(
            "CREATE TABLE archive_batches (
                id INTEGER PRIMARY KEY,
                file_path TEXT NOT NULL,
                replacement_staged_path TEXT,
                sha256 TEXT
            )",
        )
        .execute(&pool)
        .await
        .expect("create unverified staged replacement manifest");
        let root = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-staged-unverified-{}-{}",
            std::process::id(),
            retention_temp_suffix()
        ));
        fs::create_dir_all(&root).expect("create unverified staged directory");
        let final_path = root.join("archive.sqlite.gz");
        let staged_path = root.join("archive.sqlite.gz.restore");
        fs::write(&final_path, b"replacement artifact").expect("write replacement artifact");
        fs::write(&staged_path, b"corrupt rollback artifact").expect("write corrupt rollback");
        let expected_old_path = root.join("expected-old-bytes");
        fs::write(&expected_old_path, b"previous artifact").expect("write expected old bytes");
        let old_sha = sha256_hex_file(&expected_old_path).expect("hash expected old bytes");
        fs::remove_file(&expected_old_path).expect("remove expected old bytes");
        let file_path = final_path.to_string_lossy().to_string();
        let staged_path_string = staged_path.to_string_lossy().to_string();
        sqlx::query(
            "INSERT INTO archive_batches (id, file_path, replacement_staged_path, sha256) VALUES (1, ?1, ?2, ?3)",
        )
        .bind(&file_path)
        .bind(&staged_path_string)
        .bind(&old_sha)
        .execute(&pool)
        .await
        .expect("seed corrupt staged replacement manifest");

        reconcile_staged_archive_replacements(&pool, None)
            .await
            .expect("reconcile should retain unverified staged replacement");
        assert_eq!(
            fs::read(&final_path).expect("read unchanged replacement artifact"),
            b"replacement artifact"
        );
        let staged: Option<String> =
            sqlx::query_scalar("SELECT replacement_staged_path FROM archive_batches WHERE id = 1")
                .fetch_one(&pool)
                .await
                .expect("load retained replacement staging path");
        assert_eq!(staged.as_deref(), Some(staged_path_string.as_str()));
        let _ = fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn staged_archive_replacement_reconcile_does_not_decode_or_delete_null_digest() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect null digest replacement database");
        sqlx::query(
            "CREATE TABLE archive_batches (
                id INTEGER PRIMARY KEY,
                file_path TEXT NOT NULL,
                replacement_staged_path TEXT,
                sha256 TEXT
            )",
        )
        .execute(&pool)
        .await
        .expect("create null digest replacement manifest");
        let root = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-staged-null-{}-{}",
            std::process::id(),
            retention_temp_suffix()
        ));
        fs::create_dir_all(&root).expect("create null digest staged directory");
        let final_path = root.join("archive.sqlite.gz");
        let staged_path = root.join("archive.sqlite.gz.restore");
        fs::write(&final_path, b"current artifact").expect("write current artifact");
        fs::write(&staged_path, b"unknown rollback artifact").expect("write unknown rollback");
        let file_path = final_path.to_string_lossy().to_string();
        let staged_path_string = staged_path.to_string_lossy().to_string();
        sqlx::query(
            "INSERT INTO archive_batches (id, file_path, replacement_staged_path, sha256) VALUES (1, ?1, ?2, NULL)",
        )
        .bind(&file_path)
        .bind(&staged_path_string)
        .execute(&pool)
        .await
        .expect("seed null digest replacement manifest");

        reconcile_staged_archive_replacements(&pool, None)
            .await
            .expect("reconcile should retain null digest replacement");
        assert_eq!(
            fs::read(&final_path).expect("read unchanged current artifact"),
            b"current artifact"
        );
        assert!(staged_path.exists());
        let staged: Option<String> =
            sqlx::query_scalar("SELECT replacement_staged_path FROM archive_batches WHERE id = 1")
                .fetch_one(&pool)
                .await
                .expect("load retained null digest staging path");
        assert_eq!(staged.as_deref(), Some(staged_path_string.as_str()));
        let _ = fs::remove_dir_all(&root);
    }
}

#[derive(Debug, FromRow)]
pub(crate) struct ArchiveExpiryBackfillCandidate {
    pub(crate) id: i64,
    pub(crate) coverage_end_at: String,
}
