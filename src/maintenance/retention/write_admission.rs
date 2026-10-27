use super::*;

pub(crate) const RETENTION_FAIRNESS_INTERVAL: Duration = Duration::from_secs(15);
pub(crate) const RETENTION_WRITE_ADMISSION_BUDGET: Duration = Duration::from_secs(15);
pub(crate) const RETENTION_PRESSURE_RETRY_MAX_ATTEMPTS: usize = 3;
pub(crate) const RETENTION_PRESSURE_RETRY_BUDGET: Duration = Duration::from_secs(45);
pub(crate) const RETENTION_ARCHIVE_FILE_OPERATION_BUDGET: Duration = Duration::from_secs(15);
pub(crate) const RETENTION_ARCHIVE_FILE_OPERATION_BUDGET_REASON: &str =
    "retention_archive_file_operation_budget";
pub(crate) const RETENTION_ARCHIVE_FILE_NO_PROGRESS_REASON: &str =
    "retention_archive_file_no_progress";
pub(crate) const RETENTION_WRITE_TARGET: Duration = Duration::from_millis(200);
pub(crate) const RETENTION_WRITE_WARNING: Duration = Duration::from_millis(250);
pub(crate) const RETENTION_WRITE_INITIAL_ROWS: usize = 4;
pub(crate) const RETENTION_WRITE_MAX_ROWS: usize = 64;
pub(crate) const RETENTION_WRITE_MAX_BYTES: usize = 1024 * 1024;
pub(crate) const RETENTION_SQLITE_MAINTENANCE_QUERY_BUDGET: Duration = Duration::from_secs(2);
pub(crate) const RETENTION_BACKLOG_OBSERVER_INTERVAL: Duration = Duration::from_secs(5 * 60);
pub(crate) const RETENTION_BACKLOG_OBSERVER_QUERY_BUDGET: Duration = Duration::from_secs(2);
pub(crate) const RETENTION_BACKLOG_OBSERVER_PROGRESS_OPS: i32 = 1_000;
pub(crate) const SYSTEM_TASK_RUN_RETENTION_KEEP_RECENT: i64 = 200;
pub(crate) const SYSTEM_TASK_RUN_RETENTION_TERMINAL_BATCH_ROWS: usize = 500;
pub(crate) const SYSTEM_TASK_RUN_RETENTION_MAX_ROWS_PER_PASS: usize = 5_000;
pub(crate) const SYSTEM_TASK_RUN_RETENTION_PASS_INTERVAL: Duration = Duration::from_secs(15);
pub(crate) const SYSTEM_TASK_RUN_RETENTION_PRESSURE_BACKOFF: Duration = Duration::from_secs(5 * 60);
pub(crate) const RETENTION_RECOVERY_LEGACY_SCAN_BATCH: usize = 32;
pub(crate) const RETENTION_RAW_RECONCILIATION_SCAN_BATCH: usize = 32;
pub(crate) const RETENTION_RAW_RECONCILIATION_PENDING_BATCH: usize = 8;
pub(crate) const RETENTION_RAW_RECONCILIATION_DIRECTORY_ENTRY_BATCH: usize = 128;
pub(crate) const RETENTION_RAW_RECONCILIATION_SCOPE: &str = "raw_payload_files";
pub(crate) const RETENTION_RAW_RECONCILIATION_PROGRESS_RETRY_SECS: i64 = 1;
pub(crate) const RETENTION_RAW_RECONCILIATION_PRESSURE_RETRY_SECS: i64 = 1;
pub(crate) const RETENTION_RAW_RECONCILIATION_EMPTY_RETRY_SECS: i64 = 5 * 60;
pub(crate) const RETENTION_RAW_RECONCILIATION_CANDIDATE_BUDGET: Duration = Duration::from_secs(2);
pub(crate) const RETENTION_RAW_RECONCILIATION_FAILURE_BACKOFF_SECS: [i64; 5] =
    [5 * 60, 10 * 60, 20 * 60, 40 * 60, 60 * 60];
pub(crate) const RETENTION_RECOVERY_QUARANTINE_GRACE_SECS: i64 = 24 * 60 * 60;
pub(crate) const RETENTION_RECOVERY_STATE_PREPARING: &str = "preparing";
pub(crate) const RETENTION_RECOVERY_STATE_PUBLISHED: &str = "published";
pub(crate) const RETENTION_RECOVERY_STATE_QUARANTINED: &str = "quarantined";
pub(crate) const RETENTION_RECOVERY_PUBLICATION_DETAIL_PRUNE: &str = "detail_prune";
pub(crate) const RETENTION_RECOVERY_PUBLICATION_INVOCATION_ARCHIVE: &str = "invocation_archive";
pub(crate) const RETENTION_RECOVERY_PREPARED_SCOPE: &str = "prepared_archives";
pub(crate) const RETENTION_RECOVERY_DEFER_SQLITE_PRESSURE: &str = "sqlite_pressure";
pub(crate) const RETENTION_RECOVERY_DEFER_RETRY_BACKOFF: &str = "retry_backoff";
pub(crate) const RETENTION_RECOVERY_PRESSURE_RETRY_SECS: i64 = 5 * 60;
pub(crate) const RETENTION_RECOVERY_PROGRESS_RETRY_SECS: i64 = 15;
pub(crate) const RETENTION_RECOVERY_EMPTY_RETRY_SECS: i64 = 5 * 60;
pub(crate) const RETENTION_RECOVERY_FAILURE_BACKOFF_SECS: [i64; 5] =
    [5 * 60, 10 * 60, 20 * 60, 40 * 60, 60 * 60];

pub(crate) static SYSTEM_TASK_RUN_RETENTION_NEXT_PASS_EPOCH_MS: AtomicU64 = AtomicU64::new(0);
pub(crate) static SYSTEM_TASK_RUN_RETENTION_SCHEDULE_LOCK: std::sync::Mutex<()> =
    std::sync::Mutex::new(());

tokio::task_local! {
    pub(crate) static RETENTION_SHUTDOWN: CancellationToken;
    pub(crate) static RETENTION_MAINTENANCE_ACTIVE: ();
    pub(crate) static RETENTION_RUN_DEADLINE: RefCell<Option<Instant>>;
    pub(crate) static RETENTION_RUN_DEFER_REASON: RefCell<Option<String>>;
    pub(crate) static RETENTION_CURRENT_PREPARED_KEY: RefCell<Option<String>>;
    pub(crate) static RETENTION_TRY_ARCHIVE_LOCKS: ();
    pub(crate) static RETENTION_RAW_CAPTURE_CIRCUIT: RefCell<Option<Arc<RawCaptureCircuitBreaker>>>;
}

pub(crate) async fn mark_retention_raw_inventory_reset_intent(pool: &Pool<Sqlite>) -> Result<()> {
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
    run_data_retention_maintenance_with_circuit_and_prompt_cache(
        pool, config, dry_run, shutdown, circuit, None, None,
    )
    .await
}

pub(crate) async fn retention_try_archive_locks_scope<F: Future>(future: F) -> F::Output {
    RETENTION_TRY_ARCHIVE_LOCKS.scope((), future).await
}

pub(crate) fn retention_archive_locks_are_try_only() -> bool {
    RETENTION_TRY_ARCHIVE_LOCKS.try_with(|_| ()).is_ok()
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct RetentionSqliteMaintenanceTestProbe {
    pub(crate) progress_callbacks: std::sync::atomic::AtomicUsize,
    pub(crate) connection_closed: AtomicBool,
}

#[cfg(test)]
tokio::task_local! {
    pub(crate) static RETENTION_TEST_SQLITE_MAINTENANCE_PROBE:
        Arc<RetentionSqliteMaintenanceTestProbe>;
    pub(crate) static RETENTION_TEST_WRITE_COORDINATOR:
        std::sync::Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>;
    pub(crate) static RETENTION_TEST_WRITE_ADMISSION_BUDGET: RefCell<Option<Duration>>;
    pub(crate) static RETENTION_TEST_ARCHIVE_FILE_OPERATION_BUDGET: RefCell<Option<Duration>>;
    pub(crate) static RETENTION_TEST_DB_PRESSURE_GATE:
        std::sync::Arc<crate::db_pressure::DbPressureGate>;
    pub(crate) static RETENTION_TEST_WRITE_CONNECTION_POOL_READY: Arc<Notify>;
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
    pub(crate) static RETENTION_TEST_RAW_DIRECTORY_OPENS:
        std::sync::Arc<std::sync::atomic::AtomicUsize>;
    pub(crate) static RETENTION_TEST_RAW_DIRECTORY_ENTRIES:
        std::sync::Arc<std::sync::atomic::AtomicUsize>;
    pub(crate) static RETENTION_TEST_RAW_DIRECTORY_PRESSURE_PROBES:
        std::sync::Arc<std::sync::atomic::AtomicUsize>;
    pub(crate) static RETENTION_TEST_RAW_DIRECTORY_P1_PROBES:
        std::sync::Arc<std::sync::atomic::AtomicUsize>;
    pub(crate) static RETENTION_TEST_RAW_FILE_METADATA_CHECKS:
        std::sync::Arc<std::sync::atomic::AtomicUsize>;
    pub(crate) static RETENTION_TEST_RAW_UNLINK_FAILURE:
        std::sync::Arc<std::sync::atomic::AtomicBool>;
    pub(crate) static RETENTION_TEST_RAW_LEDGER_CLEANUP_FAILURE:
        std::sync::Arc<std::sync::atomic::AtomicBool>;
    pub(crate) static RETENTION_TEST_RAW_REMOVAL_EVIDENCE_FAILURE:
        std::sync::Arc<std::sync::atomic::AtomicBool>;
    pub(crate) static RETENTION_TEST_RAW_MIDPASS_WRITE_DENY:
        std::sync::Arc<std::sync::atomic::AtomicBool>;
    pub(crate) static RETENTION_TEST_BACKLOG_OBSERVATION_FAILURE: ();
    pub(crate) static RETENTION_TEST_INVOCATION_STAGE_TRACE:
        std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>;
}

#[cfg(test)]
pub(crate) fn retention_test_record_invocation_stage(stage: &'static str) {
    let _ = RETENTION_TEST_INVOCATION_STAGE_TRACE.try_with(|trace| {
        trace.lock().expect("retention stage trace").push(stage);
    });
}

#[cfg(test)]
pub(crate) fn retention_test_legacy_directory_traversal_event() {
    let _ = RETENTION_TEST_LEGACY_DIRECTORY_TRAVERSAL.try_with(|counter| {
        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    });
}

#[cfg(test)]
pub(crate) fn retention_test_legacy_directory_entry_event() {
    let _ = RETENTION_TEST_LEGACY_DIRECTORY_ENTRIES.try_with(|counter| {
        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    });
}

#[cfg(test)]
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct RetentionTestLegacyDirectoryHeapEntryGuard {
    pub(crate) observed: bool,
}

#[cfg(test)]
impl RetentionTestLegacyDirectoryHeapEntryGuard {
    pub(crate) fn new() -> Self {
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
pub(crate) fn retention_test_legacy_archive_io_event() {
    let _ = RETENTION_TEST_LEGACY_ARCHIVE_IO.try_with(|counter| {
        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    });
}

#[cfg(test)]
pub(crate) fn retention_test_raw_directory_open_event() {
    let _ = RETENTION_TEST_RAW_DIRECTORY_OPENS.try_with(|counter| {
        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    });
}

#[cfg(test)]
pub(crate) fn retention_test_raw_directory_entry_event() {
    let _ = RETENTION_TEST_RAW_DIRECTORY_ENTRIES.try_with(|counter| {
        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    });
    let _ = RETENTION_TEST_RAW_DIRECTORY_PRESSURE_PROBES.try_with(|counter| {
        let pressure_gate = RETENTION_TEST_DB_PRESSURE_GATE
            .try_with(std::sync::Arc::clone)
            .ok();
        if pressure_gate.is_some_and(|gate| {
            gate.try_begin_background("raw_sweep_directory_probe")
                .is_err()
        }) {
            counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    });
    let _ = RETENTION_TEST_RAW_DIRECTORY_P1_PROBES.try_with(|counter| {
        let coordinator = RETENTION_TEST_WRITE_COORDINATOR
            .try_with(std::sync::Arc::clone)
            .ok();
        if coordinator.is_some_and(|coordinator| {
            coordinator
                .try_acquire(
                    crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal,
                )
                .is_some()
        }) {
            counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    });
}

pub(crate) fn retention_raw_file_metadata(path: &Path) -> io::Result<fs::Metadata> {
    #[cfg(test)]
    let _ = RETENTION_TEST_RAW_FILE_METADATA_CHECKS.try_with(|counter| {
        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    });
    fs::symlink_metadata(path)
}

#[cfg(test)]
pub(crate) fn retention_test_raw_unlink_should_fail() -> bool {
    RETENTION_TEST_RAW_UNLINK_FAILURE
        .try_with(|flag| flag.swap(false, std::sync::atomic::Ordering::Relaxed))
        .unwrap_or(false)
}

#[cfg(test)]
pub(crate) fn retention_test_raw_ledger_cleanup_should_fail() -> bool {
    RETENTION_TEST_RAW_LEDGER_CLEANUP_FAILURE
        .try_with(|flag| flag.swap(false, std::sync::atomic::Ordering::Relaxed))
        .unwrap_or(false)
}

#[cfg(test)]
pub(crate) fn retention_test_raw_removal_evidence_should_fail() -> bool {
    RETENTION_TEST_RAW_REMOVAL_EVIDENCE_FAILURE
        .try_with(|flag| flag.swap(false, std::sync::atomic::Ordering::Relaxed))
        .unwrap_or(false)
}

pub(crate) static RETENTION_DEFER_GENERATION: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// Advisory directory lock shared by archive publishers and cleanup finalizers. SQLite admission
/// serializes database writers, while this lock also fences their filesystem rename/delete window.
#[cfg(unix)]
#[derive(Debug)]
pub(crate) struct RetentionArchiveFileLock(Option<File>);

#[cfg(unix)]
impl RetentionArchiveFileLock {
    pub(crate) fn is_held(&self) -> bool {
        self.0.is_some()
    }
}

struct ArchiveLockCancellationSignal(Arc<AtomicBool>);

impl Drop for ArchiveLockCancellationSignal {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[cfg(unix)]
pub(crate) fn retention_file_lock(
    path: &Path,
    lock_mode: libc::c_int,
    try_only: bool,
) -> Result<RetentionArchiveFileLock> {
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
    let lock_flags = if try_only {
        lock_mode | libc::LOCK_NB
    } else {
        lock_mode
    };
    let result = unsafe { libc::flock(file.as_raw_fd(), lock_flags) };
    if result != 0 {
        if try_only {
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

#[cfg(unix)]
pub(crate) fn retention_archive_file_lock(path: &Path) -> Result<RetentionArchiveFileLock> {
    retention_file_lock(path, libc::LOCK_EX, retention_archive_locks_are_try_only())
}

#[cfg(unix)]
fn retention_file_lock_until(
    path: &Path,
    deadline: Instant,
    cancellation: &AtomicBool,
    shutdown: Option<&CancellationToken>,
    try_only: bool,
) -> Result<RetentionArchiveFileLock> {
    if try_only {
        return retention_file_lock(path, libc::LOCK_EX, true);
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("archive path has no parent directory"))?;
    let file = match File::open(parent) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
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
    loop {
        if cancellation.load(Ordering::Acquire)
            || shutdown.is_some_and(CancellationToken::is_cancelled)
        {
            return Err(retention_write_deferred("archive_file_lock"));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(retention_archive_file_operation_budget_exhausted());
        }
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result == 0 {
            return Ok(RetentionArchiveFileLock(Some(file)));
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EAGAIN)
            && error.raw_os_error() != Some(libc::EWOULDBLOCK)
        {
            return Err(error).with_context(|| {
                format!("failed to lock archive directory: {}", parent.display())
            });
        }
        std::thread::sleep(remaining.min(Duration::from_millis(10)));
    }
}

pub(crate) async fn retention_archive_file_lock_async(
    path: &Path,
) -> Result<RetentionArchiveFileLock> {
    let path = path.to_path_buf();
    let deadline = retention_archive_file_operation_deadline();
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(retention_archive_file_operation_budget_exhausted());
    }
    let try_only = retention_archive_locks_are_try_only();
    let shutdown = retention_run_shutdown_token();
    let cancellation = Arc::new(AtomicBool::new(false));
    let _cancellation_signal = ArchiveLockCancellationSignal(Arc::clone(&cancellation));
    let worker_cancellation = Arc::clone(&cancellation);
    let worker = tokio::task::spawn_blocking(move || {
        #[cfg(unix)]
        {
            retention_file_lock_until(
                &path,
                deadline,
                &worker_cancellation,
                shutdown.as_ref(),
                try_only,
            )
        }
        #[cfg(not(unix))]
        {
            retention_archive_file_lock(&path)
        }
    });
    match tokio::time::timeout(remaining, worker).await {
        Ok(joined) => joined.context("archive directory lock worker panicked")?,
        Err(_) => {
            cancellation.store(true, Ordering::Release);
            Err(retention_archive_file_operation_budget_exhausted())
        }
    }
}

#[cfg(unix)]
pub(crate) fn retention_task_work_directory_lock(path: &Path) -> Result<RetentionArchiveFileLock> {
    match retention_file_lock(path, libc::LOCK_EX, true) {
        Err(error) if error.to_string() == "archive directory lock busy" => {
            retention_record_defer("archive_work_cleanup", "archive_directory_lock_busy");
            Err(retention_write_deferred("archive_work_cleanup"))
        }
        result => result,
    }
}

pub(crate) fn retention_archive_file_try_lock(path: &Path) -> Result<RetentionArchiveFileLock> {
    #[cfg(unix)]
    {
        match retention_file_lock(path, libc::LOCK_EX, true) {
            Err(error) if error.to_string() == "archive directory lock busy" => {
                retention_record_defer("archive_file_fence", "archive_directory_lock_busy");
                Err(retention_write_deferred("archive_file_fence"))
            }
            result => result,
        }
    }
    #[cfg(not(unix))]
    {
        retention_archive_file_lock(path)
    }
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
#[derive(Debug)]
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

#[cfg(not(unix))]
pub(crate) fn retention_task_work_directory_lock(path: &Path) -> Result<RetentionArchiveFileLock> {
    retention_archive_file_lock(path)
}

pub(crate) fn acquire_retention_raw_write_fence(
    path: &Path,
) -> io::Result<std::sync::Arc<RetentionArchiveFileLock>> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("raw payload path has no parent: {}", path.display()),
        )
    })?;
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    let lock = retention_file_lock(path, libc::LOCK_SH, false);
    #[cfg(not(unix))]
    let lock = retention_archive_file_lock(path);
    lock.map(std::sync::Arc::new)
        .map_err(|error| io::Error::other(error.to_string()))
}

pub(crate) fn acquire_retention_raw_exclusive_fence(
    path: &Path,
) -> io::Result<std::sync::Arc<RetentionArchiveFileLock>> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("raw payload path has no parent: {}", path.display()),
        )
    })?;
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    let lock = retention_file_lock(path, libc::LOCK_EX, false);
    #[cfg(not(unix))]
    let lock = retention_archive_file_lock(path);
    lock.map(std::sync::Arc::new)
        .map_err(|error| io::Error::other(error.to_string()))
}

#[cfg(unix)]
pub(crate) fn sync_retention_raw_parent(path: &Path) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("raw payload path has no parent: {}", path.display()),
        )
    })?;
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
pub(crate) fn sync_retention_raw_parent(_path: &Path) -> io::Result<()> {
    Ok(())
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
    pub(crate) raw_reference_check_ms: Option<u64>,
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
            raw_reference_check_ms: None,
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
    pub(crate) defer_reason: Option<String>,
    pub(crate) consecutive_failure_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RawOrphanSweepSettledPassSnapshot {
    pub(crate) settled_at: String,
    pub(crate) complete: bool,
    pub(crate) inspected_entries: usize,
    pub(crate) referenced_skipped: usize,
    pub(crate) quarantined: usize,
    pub(crate) removed: usize,
    pub(crate) removed_bytes: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RawOrphanSweepRemovalSnapshot {
    pub(crate) removed_at: String,
    pub(crate) removed: usize,
    pub(crate) removed_bytes: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RawOrphanSweepHealthSnapshot {
    pub(crate) state: String,
    pub(crate) inspected_entries: Option<usize>,
    pub(crate) referenced_skipped: Option<usize>,
    pub(crate) quarantined: Option<usize>,
    pub(crate) removed: Option<usize>,
    pub(crate) removed_bytes: Option<u64>,
    pub(crate) last_progress_at: Option<String>,
    pub(crate) next_retry_at: Option<String>,
    pub(crate) defer_reason: Option<String>,
    pub(crate) failure_fingerprint: Option<String>,
    pub(crate) admission_stage: Option<String>,
    pub(crate) admission_cause: Option<String>,
    pub(crate) last_settled_pass: Option<RawOrphanSweepSettledPassSnapshot>,
    pub(crate) last_nonzero_removal: Option<RawOrphanSweepRemovalSnapshot>,
}

impl Default for RawOrphanSweepHealthSnapshot {
    fn default() -> Self {
        Self {
            state: "unknown".to_string(),
            inspected_entries: None,
            referenced_skipped: None,
            quarantined: None,
            removed: None,
            removed_bytes: None,
            last_progress_at: None,
            next_retry_at: None,
            defer_reason: None,
            failure_fingerprint: None,
            admission_stage: None,
            admission_cause: None,
            last_settled_pass: None,
            last_nonzero_removal: None,
        }
    }
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
            defer_reason: None,
            consecutive_failure_count: None,
        }
    }
}

pub(crate) static RETENTION_RECOVERY_HEALTH: Lazy<
    std::sync::Mutex<RetentionRecoveryHealthSnapshot>,
> = Lazy::new(|| std::sync::Mutex::new(RetentionRecoveryHealthSnapshot::default()));
pub(crate) static RAW_ORPHAN_SWEEP_HEALTH: Lazy<std::sync::Mutex<RawOrphanSweepHealthSnapshot>> =
    Lazy::new(|| std::sync::Mutex::new(RawOrphanSweepHealthSnapshot::default()));

pub(crate) fn retention_recovery_health_snapshot() -> RetentionRecoveryHealthSnapshot {
    RETENTION_RECOVERY_HEALTH
        .lock()
        .expect("retention recovery health")
        .clone()
}

pub(crate) fn raw_orphan_sweep_health_snapshot() -> RawOrphanSweepHealthSnapshot {
    RAW_ORPHAN_SWEEP_HEALTH
        .lock()
        .expect("raw orphan sweep health")
        .clone()
}

pub(crate) fn retention_recovery_set_stage(stage: &'static str) {
    let mut health = RETENTION_RECOVERY_HEALTH
        .lock()
        .expect("retention recovery health");
    if !(stage != "prepared_reconcile" && retention_recovery_prepared_stage_is_blocking(&health)) {
        health.stage = Some(stage.to_string());
    }
    if health.state == "unknown" {
        health.state = "recovering".to_string();
    }
}

pub(crate) fn retention_recovery_prepared_stage_is_blocking(
    health: &RetentionRecoveryHealthSnapshot,
) -> bool {
    health.stage.as_deref() == Some("prepared_reconcile")
        && (health.failure_stage.as_deref() == Some("prepared_reconcile")
            || matches!(
                health.defer_reason.as_deref(),
                Some(RETENTION_RECOVERY_DEFER_SQLITE_PRESSURE)
                    | Some(RETENTION_RECOVERY_DEFER_RETRY_BACKOFF)
            ))
}

pub(crate) fn retention_recovery_record_progress() {
    let mut health = RETENTION_RECOVERY_HEALTH
        .lock()
        .expect("retention recovery health");
    health.state = "healthy".to_string();
    health.last_progress_at = Some(format_utc_iso(Utc::now()));
    health.defer_reason = None;
    health.consecutive_failure_count = Some(0);
}

pub(crate) fn retention_recovery_set_current_prepared_key(prepared_key: &str) {
    let _ = RETENTION_CURRENT_PREPARED_KEY.try_with(|current| {
        *current.borrow_mut() = Some(prepared_key.to_string());
    });
}

pub(crate) fn retention_recovery_current_prepared_key() -> Option<String> {
    RETENTION_CURRENT_PREPARED_KEY
        .try_with(|current| current.borrow().clone())
        .ok()
        .flatten()
}

pub(crate) fn retention_recovery_clear_current_prepared_key() {
    let _ = RETENTION_CURRENT_PREPARED_KEY.try_with(|current| {
        *current.borrow_mut() = None;
    });
}

pub(crate) fn retention_run_budget_expired() -> bool {
    RETENTION_RUN_DEADLINE
        .try_with(|deadline| {
            deadline
                .borrow()
                .is_some_and(|value| Instant::now() >= value)
        })
        .unwrap_or(false)
}

pub(crate) fn retention_run_remaining_budget() -> Option<Duration> {
    RETENTION_RUN_DEADLINE
        .try_with(|deadline| {
            deadline
                .borrow()
                .map(|value| value.saturating_duration_since(Instant::now()))
        })
        .ok()
        .flatten()
}

pub(crate) fn retention_maintenance_is_active() -> bool {
    RETENTION_MAINTENANCE_ACTIVE.try_with(|_| ()).is_ok()
}

pub(crate) fn retention_prepared_archive_isolation_active() -> bool {
    retention_run_remaining_budget().is_some() || (retention_maintenance_is_active() && !cfg!(test))
}

fn configured_retention_write_admission_budget() -> Duration {
    #[cfg(test)]
    if let Ok(Some(budget)) =
        RETENTION_TEST_WRITE_ADMISSION_BUDGET.try_with(|value| *value.borrow())
    {
        return budget;
    }

    RETENTION_WRITE_ADMISSION_BUDGET
}

pub(crate) fn retention_write_admission_budget() -> (Duration, &'static str) {
    let configured = configured_retention_write_admission_budget();
    match retention_run_remaining_budget() {
        Some(remaining) if remaining <= configured => (remaining, "retention_work_budget"),
        _ => (configured, "retention_write_admission"),
    }
}

fn configured_retention_archive_file_operation_budget() -> Duration {
    #[cfg(test)]
    if let Ok(Some(budget)) =
        RETENTION_TEST_ARCHIVE_FILE_OPERATION_BUDGET.try_with(|value| *value.borrow())
    {
        return budget;
    }

    RETENTION_ARCHIVE_FILE_OPERATION_BUDGET
}

pub(crate) fn retention_archive_file_operation_deadline() -> Instant {
    Instant::now() + configured_retention_archive_file_operation_budget()
}

pub(crate) fn retention_archive_file_operation_no_progress(
    processed_bytes: u64,
    total_input_bytes: Option<u64>,
) -> anyhow::Error {
    anyhow!(
        "{RETENTION_ARCHIVE_FILE_NO_PROGRESS_REASON}: processed_bytes={processed_bytes}; total_input_bytes={total_input_bytes:?}"
    )
}

pub(crate) fn is_retention_archive_file_operation_no_progress(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .to_string()
            .contains(RETENTION_ARCHIVE_FILE_NO_PROGRESS_REASON)
    })
}

pub(crate) fn retention_run_shutdown_token() -> Option<CancellationToken> {
    RETENTION_SHUTDOWN.try_with(Clone::clone).ok()
}

pub(crate) fn retention_run_defer_reason() -> Option<String> {
    RETENTION_RUN_DEFER_REASON
        .try_with(|reason| reason.borrow().clone())
        .ok()
        .flatten()
}

#[cfg(test)]
pub(crate) async fn retention_test_with_work_budget<F: std::future::Future>(
    budget: Duration,
    work: F,
) -> F::Output {
    RETENTION_RUN_DEADLINE
        .scope(RefCell::new(Some(Instant::now() + budget)), work)
        .await
}

#[cfg(test)]
pub(crate) async fn retention_test_with_write_admission_budget<F: Future>(
    budget: Duration,
    work: F,
) -> F::Output {
    RETENTION_TEST_WRITE_ADMISSION_BUDGET
        .scope(RefCell::new(Some(budget)), work)
        .await
}

#[cfg(test)]
pub(crate) async fn retention_test_with_archive_file_operation_budget<F: Future>(
    budget: Duration,
    work: F,
) -> F::Output {
    RETENTION_TEST_ARCHIVE_FILE_OPERATION_BUDGET
        .scope(RefCell::new(Some(budget)), work)
        .await
}

#[cfg(test)]
pub(crate) fn retention_test_expire_work_budget() {
    RETENTION_RUN_DEADLINE.with(|deadline| {
        *deadline.borrow_mut() = Some(Instant::now());
    });
}

#[cfg(test)]
pub(crate) async fn retention_test_with_shutdown<F: Future>(
    shutdown: CancellationToken,
    work: F,
) -> F::Output {
    RETENTION_SHUTDOWN.scope(shutdown, work).await
}

#[cfg(test)]
pub(crate) async fn retention_test_with_recorded_defer_reason<F: Future>(
    work: F,
) -> (F::Output, Option<String>) {
    RETENTION_RUN_DEFER_REASON
        .scope(RefCell::new(None), async {
            let result = work.await;
            let reason = RETENTION_RUN_DEFER_REASON.with(|reason| reason.borrow().clone());
            (result, reason)
        })
        .await
}

pub(crate) fn retention_recovery_record_failure(stage: &'static str, error: &anyhow::Error) {
    let fingerprint = retention_error_fingerprint(error);
    let mut health = RETENTION_RECOVERY_HEALTH
        .lock()
        .expect("retention recovery health");
    health.state = "degraded".to_string();
    health.failure_stage = Some(stage.to_string());
    health.failure_fingerprint = Some(fingerprint);
    health.defer_reason = Some(RETENTION_RECOVERY_DEFER_RETRY_BACKOFF.to_string());
}

pub(crate) fn retention_error_fingerprint(error: &anyhow::Error) -> String {
    let digest = Sha256::digest(error.to_string().as_bytes());
    format!("{:x}", digest)[..16].to_string()
}

pub(crate) fn retention_recovery_record_deferred(stage: &'static str) {
    let mut health = RETENTION_RECOVERY_HEALTH
        .lock()
        .expect("retention recovery health");
    health.state = "deferred".to_string();
    if !(stage != "prepared_reconcile" && retention_recovery_prepared_stage_is_blocking(&health)) {
        health.stage = Some(stage.to_string());
    }
    health.defer_reason = Some(RETENTION_RECOVERY_DEFER_SQLITE_PRESSURE.to_string());
    health.next_retry_at = Some(format_utc_iso(
        Utc::now() + chrono::Duration::seconds(RETENTION_RECOVERY_PRESSURE_RETRY_SECS),
    ));
}

pub(crate) fn retention_recovery_log_event(
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
                retention_recovery_defer_reason = ?health.defer_reason,
                retention_recovery_consecutive_failure_count = ?health.consecutive_failure_count,
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
pub(crate) struct RetentionWriteBudget {
    pub(crate) next_rows: usize,
    pub(crate) estimated_bytes_per_row: usize,
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
    pub(crate) fn candidate_limit(&self, configured_limit: usize) -> usize {
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

    pub(crate) fn observe_commit(
        &mut self,
        rows: usize,
        estimated_bytes: usize,
        elapsed: Duration,
    ) -> bool {
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
            // A single oversized row must not leave the byte estimate high enough to force every
            // later small commit through a one-row candidate. The actual micro-batch builder
            // still enforces RETENTION_WRITE_MAX_BYTES for every candidate, so this only removes
            // stale conservative state after a successful normal commit.
            self.estimated_bytes_per_row = self
                .estimated_bytes_per_row
                .min(observed_bytes_per_row.saturating_mul(4).max(256));
            self.next_rows = self
                .next_rows
                .saturating_add(1)
                .min(RETENTION_WRITE_MAX_ROWS);
        }
        breached
    }
}

#[derive(Debug, Default)]
pub(crate) struct RetentionWriteHealthState {
    pub(crate) snapshot: RetentionWriteHealthSnapshot,
    pub(crate) budgets: HashMap<&'static str, RetentionWriteBudget>,
}

pub(crate) static RETENTION_WRITE_HEALTH: Lazy<std::sync::Mutex<RetentionWriteHealthState>> =
    Lazy::new(|| std::sync::Mutex::new(RetentionWriteHealthState::default()));

#[derive(Debug)]
pub(crate) struct RetentionArchiveFileOperationBudgetExceeded;

impl std::fmt::Display for RetentionArchiveFileOperationBudgetExceeded {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("retention archive file work exceeded the operation budget")
    }
}

impl std::error::Error for RetentionArchiveFileOperationBudgetExceeded {}

pub(crate) fn retention_archive_file_operation_budget_exhausted() -> anyhow::Error {
    anyhow::Error::new(RetentionArchiveFileOperationBudgetExceeded)
}

pub(crate) fn is_retention_archive_file_operation_budget_exhausted(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.is::<RetentionArchiveFileOperationBudgetExceeded>())
}

pub(crate) fn is_retention_archive_file_work_transient(error: &anyhow::Error) -> bool {
    is_retention_archive_file_operation_budget_exhausted(error)
        || is_retention_write_deferred(error)
}

#[derive(Debug)]
pub(crate) struct RetentionWriteDeferred {
    pub(crate) operation: &'static str,
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

pub(crate) fn retention_write_deferred(operation: &'static str) -> anyhow::Error {
    anyhow::Error::new(RetentionWriteDeferred { operation })
}

pub(crate) fn retention_write_deferred_operation(error: &anyhow::Error) -> Option<&'static str> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<RetentionWriteDeferred>())
        .map(|deferred| deferred.operation)
}

pub(crate) fn is_retention_write_deferred(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.is::<RetentionWriteDeferred>() || cause.is::<RawOrphanSweepAdmissionDeferred>()
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RawOrphanSweepAdmissionDeferred {
    pub(crate) stage: &'static str,
    pub(crate) cause: &'static str,
}

impl std::fmt::Display for RawOrphanSweepAdmissionDeferred {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("raw orphan sweep admission deferred")
    }
}

impl std::error::Error for RawOrphanSweepAdmissionDeferred {}

pub(crate) fn raw_orphan_sweep_admission_deferred(
    stage: &'static str,
    cause: &'static str,
) -> anyhow::Error {
    anyhow::Error::new(RawOrphanSweepAdmissionDeferred { stage, cause })
}

pub(crate) fn raw_orphan_sweep_admission_details(
    error: &anyhow::Error,
) -> Option<(&'static str, &'static str)> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<RawOrphanSweepAdmissionDeferred>())
        .map(|deferred| (deferred.stage, deferred.cause))
}

#[derive(Debug)]
pub(crate) struct RetentionRecoveryFailurePersisted {
    pub(crate) prepared_key: String,
    pub(crate) source: anyhow::Error,
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

pub(crate) fn retention_prepared_batch_or_deferred<T>(result: Result<T>) -> Result<Option<T>> {
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

pub(crate) fn retention_candidate_limit(config: &AppConfig, operation: &'static str) -> usize {
    // Keep ordinary unit fixtures deterministic while allowing the capacity benchmark to
    // exercise the same adaptive limiter that the release build uses.
    if cfg!(test) && std::env::var_os("CVM_RETENTION_TEST_ADAPTIVE").is_none() {
        return config.retention_batch_rows;
    }
    retention_adaptive_candidate_limit(config.retention_batch_rows, operation)
}

pub(crate) fn retention_adaptive_candidate_limit(
    configured_limit: usize,
    operation: &'static str,
) -> usize {
    let mut health = RETENTION_WRITE_HEALTH
        .lock()
        .expect("retention write health");
    retention_adaptive_candidate_limit_from_state(&mut health, configured_limit, operation)
}

pub(crate) fn retention_adaptive_candidate_limit_from_state(
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

pub(crate) fn retention_micro_batch_limit(config: &AppConfig, operation: &'static str) -> usize {
    retention_candidate_limit(config, operation).min(RETENTION_WRITE_MAX_ROWS)
}

pub(crate) fn retention_record_defer(operation: &'static str, reason: impl ToString) {
    RETENTION_DEFER_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let reason = reason.to_string();
    // Later stages may commit and clear shared health; retain this run's first actual cause.
    let first_run_defer = RETENTION_RUN_DEFER_REASON
        .try_with(|current| {
            let mut current = current.borrow_mut();
            if current.is_none() {
                *current = Some(reason.clone());
                true
            } else {
                false
            }
        })
        .unwrap_or(false);
    let mut health = RETENTION_WRITE_HEALTH
        .lock()
        .expect("retention write health");
    health.snapshot.state = "deferred".to_string();
    health.snapshot.operation = Some(operation.to_string());
    health.snapshot.defer_reason = Some(reason.clone());
    health.snapshot.raw_reference_check_ms = None;
    health.snapshot.last_error = None;
    drop(health);
    if first_run_defer {
        info!(
            operation,
            defer_reason = %reason,
            "retention run first deferred before SQLite admission"
        );
    } else {
        debug!(
            operation,
            defer_reason = %reason,
            "retention write deferred before SQLite admission"
        );
    }
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
    pub(crate) write_hold_elapsed: Option<Duration>,
    pub(crate) raw_reference_check_elapsed: Option<Duration>,
    pub(crate) p1_waiter_count: usize,
    pub(crate) candidate_remaining_hint: usize,
}

macro_rules! retention_record_commit_with_reference_check {
    (
        $operation:expr,
        $admission_mode:expr,
        $rows:expr,
        $estimated_bytes:expr,
        $prepare_elapsed:expr,
        $lock_wait:expr,
        $execute_elapsed:expr,
        $commit_elapsed:expr,
        $raw_reference_check_elapsed:expr,
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
                write_hold_elapsed: None,
                raw_reference_check_elapsed: $raw_reference_check_elapsed,
                p1_waiter_count: $p1_waiter_count,
                candidate_remaining_hint: $candidate_remaining_hint,
            },
        )
    };
}

pub(crate) use retention_record_commit_with_reference_check;

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
        $crate::maintenance::retention::retention_record_commit_with_reference_check!(
            $operation,
            $admission_mode,
            $rows,
            $estimated_bytes,
            $prepare_elapsed,
            $lock_wait,
            $execute_elapsed,
            $commit_elapsed,
            None,
            $p1_waiter_count,
            $candidate_remaining_hint,
        )
    };
}

pub(crate) use retention_record_commit;

pub(crate) fn record_retention_write_commit(commit: RetentionWriteCommit) -> bool {
    let mut health = RETENTION_WRITE_HEALTH
        .lock()
        .expect("retention write health");
    let breached = observe_retention_write_commit_with_hold(&mut health, &commit);
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
        write_hold_elapsed,
        raw_reference_check_elapsed,
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
            write_hold_ms = ?write_hold_elapsed.map(|elapsed| elapsed.as_millis() as u64),
            raw_reference_check_ms = ?raw_reference_check_elapsed.map(|elapsed| elapsed.as_millis() as u64),
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
            commit_ms = commit_elapsed.as_millis() as u64,
            write_hold_ms = ?write_hold_elapsed.map(|elapsed| elapsed.as_millis() as u64),
            raw_reference_check_ms = ?raw_reference_check_elapsed.map(|elapsed| elapsed.as_millis() as u64),
            p1_waiter_count,
            candidate_remaining_hint,
            "retention write micro-batch committed"
        );
    }
    breached
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn observe_retention_write_commit(
    health: &mut RetentionWriteHealthState,
    operation: &'static str,
    admission_mode: &'static str,
    rows: usize,
    estimated_bytes: usize,
    prepare_elapsed: Duration,
    lock_wait: Duration,
    execute_elapsed: Duration,
    commit_elapsed: Duration,
    raw_reference_check_elapsed: Option<Duration>,
    p1_waiter_count: usize,
    candidate_remaining_hint: usize,
) -> bool {
    observe_retention_write_commit_with_hold(
        health,
        &RetentionWriteCommit {
            operation,
            admission_mode,
            rows,
            estimated_bytes,
            prepare_elapsed,
            lock_wait,
            execute_elapsed,
            commit_elapsed,
            write_hold_elapsed: None,
            raw_reference_check_elapsed,
            p1_waiter_count,
            candidate_remaining_hint,
        },
    )
}

pub(crate) fn observe_retention_write_commit_with_hold(
    health: &mut RetentionWriteHealthState,
    commit: &RetentionWriteCommit,
) -> bool {
    let RetentionWriteCommit {
        operation,
        admission_mode,
        rows,
        estimated_bytes,
        prepare_elapsed,
        lock_wait,
        execute_elapsed,
        commit_elapsed,
        write_hold_elapsed,
        raw_reference_check_elapsed,
        p1_waiter_count,
        candidate_remaining_hint,
    } = *commit;
    let elapsed =
        write_hold_elapsed.unwrap_or_else(|| execute_elapsed.saturating_add(commit_elapsed));
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
    health.snapshot.raw_reference_check_ms =
        raw_reference_check_elapsed.map(|elapsed| elapsed.as_millis() as u64);
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

pub(crate) fn retention_record_error(operation: &'static str, error: &anyhow::Error) {
    let mut health = RETENTION_WRITE_HEALTH
        .lock()
        .expect("retention write health");
    health.snapshot.state = "degraded".to_string();
    health.snapshot.operation = Some(operation.to_string());
    health.snapshot.raw_reference_check_ms = None;
    health.snapshot.last_error = Some(format!(
        "failure_fingerprint:{}",
        retention_error_fingerprint(error)
    ));
}

pub(crate) fn take_retention_micro_batch<T>(
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

pub(crate) struct RetentionWriteAdmission {
    pub(crate) write_permit: crate::proxy_sqlite_write_coordinator::ProxySqliteWritePermit,
    pub(crate) _pressure_permit: crate::db_pressure::DbBackgroundPermit,
    pub(crate) p1_waiter_count: usize,
    // Fields drop in declaration order, so the observation includes permit release.
    pub(crate) _write_hold_observation: Option<observability::RetentionPhaseGuard>,
}

impl RetentionWriteAdmission {
    pub(crate) fn admission_mode(&self) -> &'static str {
        if self.write_permit.fairness_admission() {
            "fairness"
        } else {
            "normal"
        }
    }

    pub(crate) fn lock_wait(&self) -> Duration {
        self.write_permit.lock_wait()
    }

    pub(crate) fn p1_waiter_count(&self) -> usize {
        self.p1_waiter_count
    }

    pub(crate) fn write_hold_elapsed(&self) -> Duration {
        self.write_permit.held_since().elapsed()
    }

    fn release_write_permit_keep_pressure_slot(self) -> crate::db_pressure::DbBackgroundPermit {
        let Self {
            write_permit,
            _pressure_permit,
            ..
        } = self;
        drop(write_permit);
        _pressure_permit
    }
}

pub(crate) async fn acquire_retention_filesystem_pressure_slot(
    operation: &'static str,
) -> Option<crate::db_pressure::DbBackgroundPermit> {
    acquire_retention_write_admission(operation)
        .await
        .map(RetentionWriteAdmission::release_write_permit_keep_pressure_slot)
}

pub(crate) fn retention_write_coordinator_handle()
-> std::sync::Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator> {
    #[cfg(test)]
    {
        RETENTION_TEST_WRITE_COORDINATOR
            .try_with(std::sync::Arc::clone)
            .unwrap_or_else(|_| {
                crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            })
    }
    #[cfg(not(test))]
    {
        crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
    }
}

pub(crate) async fn acquire_retention_pool_connection(
    pool: &Pool<Sqlite>,
    operation: &'static str,
) -> Result<Option<sqlx::pool::PoolConnection<Sqlite>>> {
    if retention_run_budget_expired() {
        retention_record_defer(operation, "retention_work_budget");
        return Ok(None);
    }
    let shutdown = retention_run_shutdown_token();
    let acquire = async {
        if let Some(remaining) = retention_run_remaining_budget() {
            match tokio::time::timeout(remaining, pool.acquire()).await {
                Ok(result) => result.map(Some),
                Err(_) => {
                    retention_record_defer(operation, "retention_work_budget");
                    Ok(None)
                }
            }
        } else {
            let (admission_budget, timeout_reason) = retention_write_admission_budget();
            match tokio::time::timeout(admission_budget, pool.acquire()).await {
                Ok(result) => result.map(Some),
                Err(_) => {
                    retention_record_defer(operation, timeout_reason);
                    Ok(None)
                }
            }
        }
    };
    let result = if let Some(shutdown) = shutdown.as_ref() {
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => {
                retention_record_defer(operation, "shutdown");
                return Ok(None);
            }
            result = acquire => result,
        }
    } else {
        acquire.await
    };
    let connection = match result {
        Ok(connection) => connection,
        Err(sqlx::Error::PoolTimedOut) => {
            retention_record_defer(operation, "sqlite_pool_wait");
            return Ok(None);
        }
        Err(error) => return Err(error.into()),
    };
    if retention_run_budget_expired()
        || shutdown
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
    {
        drop(connection);
        retention_record_defer(
            operation,
            if retention_run_budget_expired() {
                "retention_work_budget"
            } else {
                "shutdown"
            },
        );
        return Ok(None);
    }
    Ok(connection)
}

pub(crate) async fn acquire_retention_write_connection(
    pool: &Pool<Sqlite>,
    operation: &'static str,
) -> Result<Option<(sqlx::pool::PoolConnection<Sqlite>, RetentionWriteAdmission)>> {
    batch_admission::acquire_write_connection(pool, operation, false).await
}

pub(crate) async fn acquire_retention_batch_write_connection_with_archive_fence(
    pool: &Pool<Sqlite>,
    operation: &'static str,
    archive_path: &Path,
    expected_sha256: Option<&str>,
) -> Result<
    Option<(
        sqlx::pool::PoolConnection<Sqlite>,
        RetentionWriteAdmission,
        RetentionArchiveFileLock,
    )>,
> {
    batch_admission::acquire_write_connection_with_archive_fence(
        pool,
        operation,
        archive_path,
        expected_sha256,
    )
    .await
}

pub(crate) async fn acquire_retention_batch_write_connection_with_archive_fence_observed<F>(
    pool: &Pool<Sqlite>,
    operation: &'static str,
    archive_path: &Path,
    expected_sha256: Option<&str>,
    observe_file_verify: F,
) -> Result<
    Option<(
        sqlx::pool::PoolConnection<Sqlite>,
        RetentionWriteAdmission,
        RetentionArchiveFileLock,
    )>,
>
where
    F: FnMut() -> Option<observability::RetentionPhaseGuard>,
{
    batch_admission::acquire_write_connection_with_archive_fence_observed(
        pool,
        operation,
        archive_path,
        expected_sha256,
        observe_file_verify,
    )
    .await
}

pub(crate) async fn acquire_retention_write_admission(
    operation: &'static str,
) -> Option<RetentionWriteAdmission> {
    batch_admission::acquire_write_admission(operation, false).await
}

pub(crate) async fn acquire_retention_write_coordinator(
    operation: &'static str,
) -> Option<(
    crate::proxy_sqlite_write_coordinator::ProxySqliteWritePermit,
    crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinatorSnapshot,
)> {
    if retention_run_budget_expired() {
        retention_record_defer(operation, "retention_work_budget");
        return None;
    }
    let coordinator = retention_write_coordinator_handle();
    let acquire = async {
        match RETENTION_SHUTDOWN.try_with(Clone::clone) {
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
        }
    };
    // A queued maintenance waiter owns no SQLite work. Dropping it unregisters the waiter;
    // the operation budget must also cover admission behind an active foreground writer.
    let (admission_budget, timeout_reason) = retention_write_admission_budget();
    let write_permit = match tokio::time::timeout(admission_budget, acquire).await {
        Ok(permit) => permit,
        Err(_) => {
            retention_record_defer(operation, timeout_reason);
            return None;
        }
    };
    let Some(write_permit) = write_permit else {
        retention_record_defer(operation, "shutdown");
        return None;
    };
    if retention_run_budget_expired() {
        drop(write_permit);
        retention_record_defer(operation, "retention_work_budget");
        return None;
    }
    Some((write_permit, coordinator.snapshot().await))
}
