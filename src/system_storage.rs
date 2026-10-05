use crate::{AppConfig, AppState, resolved_archive_dir};
use chrono::{DateTime, Utc};
use serde::Serialize;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{sync::Semaphore, task::JoinHandle, time::MissedTickBehavior};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

const SYSTEM_STORAGE_REFRESH_PERIOD: Duration = Duration::from_secs(60);
const SYSTEM_STORAGE_RESOURCE_BUDGET_BYTES: usize = 64 * 1024 * 1024;
const SYSTEM_STORAGE_MAX_DEPTH: usize = 128;
const SYSTEM_STORAGE_YIELD_ENTRIES: usize = 256;
const SYSTEM_STORAGE_YIELD_PERIOD: Duration = Duration::from_millis(25);
const SYSTEM_STORAGE_YIELD_DURATION: Duration = Duration::from_millis(5);
const SYSTEM_STORAGE_STALE_AFTER: Duration = Duration::from_secs(60);
const IDENTITY_BUDGET_BYTES: usize = 32;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SystemStorageState {
    Preparing,
    Ready,
    Deferred,
    Error,
    Unknown,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemStorageResponse {
    pub(crate) total_bytes: Option<u64>,
    pub(crate) sampled_at: Option<String>,
    pub(crate) state: SystemStorageState,
    pub(crate) scan_in_progress: bool,
    pub(crate) stale: bool,
    pub(crate) reason: Option<String>,
}

#[derive(Debug)]
struct StorageSnapshot {
    total_bytes: Option<u64>,
    sampled_at: Option<DateTime<Utc>>,
    state: SystemStorageState,
    scan_in_progress: bool,
    reason: Option<String>,
}

#[derive(Debug, Clone)]
struct StorageRoot {
    path: PathBuf,
    required: bool,
}

#[derive(Debug)]
pub(crate) struct SystemStorageRuntime {
    roots: Result<Vec<StorageRoot>, StorageScanFailure>,
    snapshot: tokio::sync::Mutex<StorageSnapshot>,
    scan_permit: Arc<Semaphore>,
}

impl SystemStorageRuntime {
    pub(crate) fn new(config: &AppConfig) -> Self {
        Self::from_roots(storage_roots(config))
    }

    fn from_roots(roots: Result<Vec<StorageRoot>, StorageScanFailure>) -> Self {
        Self {
            roots,
            snapshot: tokio::sync::Mutex::new(StorageSnapshot {
                total_bytes: None,
                sampled_at: None,
                state: SystemStorageState::Unknown,
                scan_in_progress: false,
                reason: None,
            }),
            scan_permit: Arc::new(Semaphore::new(1)),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_maintenance_path_for_test(
        config: &AppConfig,
        maintenance_database_path: &Path,
    ) -> Self {
        Self::from_roots(storage_roots_with_maintenance_path(
            config,
            maintenance_database_path.to_path_buf(),
        ))
    }

    pub(crate) async fn snapshot(&self) -> SystemStorageResponse {
        let snapshot = self.snapshot.lock().await;
        let stale = snapshot.sampled_at.as_ref().is_some_and(|sampled_at| {
            Utc::now()
                .signed_duration_since(sampled_at.to_owned())
                .to_std()
                .is_ok_and(|age| age > SYSTEM_STORAGE_STALE_AFTER)
        });
        SystemStorageResponse {
            total_bytes: snapshot.total_bytes,
            sampled_at: snapshot.sampled_at.as_ref().map(DateTime::to_rfc3339),
            state: snapshot.state,
            scan_in_progress: snapshot.scan_in_progress,
            stale,
            reason: snapshot.reason.clone(),
        }
    }

    async fn set_scan_started(&self) {
        let mut snapshot = self.snapshot.lock().await;
        snapshot.scan_in_progress = true;
        if snapshot.total_bytes.is_none() {
            snapshot.state = SystemStorageState::Preparing;
        }
        snapshot.reason = None;
    }

    async fn publish_result(&self, result: Result<ScanSummary, StorageScanFailure>) {
        let mut snapshot = self.snapshot.lock().await;
        snapshot.scan_in_progress = false;
        match result {
            Ok(summary) => {
                snapshot.total_bytes = Some(summary.total_bytes);
                snapshot.sampled_at = Some(Utc::now());
                snapshot.state = SystemStorageState::Ready;
                snapshot.reason = None;
                info!(
                    visited_objects = summary.visited_objects,
                    elapsed_ms = summary.elapsed.as_millis() as u64,
                    "system storage sample completed"
                );
            }
            Err(error) => {
                snapshot.state = if error.deferred {
                    SystemStorageState::Deferred
                } else if error.reason == "unsupported_platform" {
                    SystemStorageState::Unknown
                } else {
                    SystemStorageState::Error
                };
                snapshot.reason = Some(error.reason.to_string());
                warn!(
                    reason = error.reason,
                    "system storage sample failed; retaining the last successful sample"
                );
            }
        }
    }

    #[cfg(test)]
    pub(crate) async fn sample_once_for_test(&self, shutdown: &CancellationToken) {
        sample_system_storage(self, shutdown).await;
    }

    #[cfg(test)]
    pub(crate) async fn set_last_good_for_test(&self, total_bytes: u64, sampled_at: DateTime<Utc>) {
        let mut snapshot = self.snapshot.lock().await;
        snapshot.total_bytes = Some(total_bytes);
        snapshot.sampled_at = Some(sampled_at);
        snapshot.state = SystemStorageState::Ready;
    }

    #[cfg(test)]
    pub(crate) fn scan_available_permits_for_test(&self) -> usize {
        self.scan_permit.available_permits()
    }
}

pub(crate) fn spawn_system_storage_maintenance(state: Arc<AppState>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut cadence = tokio::time::interval(SYSTEM_STORAGE_REFRESH_PERIOD);
        cadence.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = state.shutdown.cancelled() => return,
                _ = cadence.tick() => {}
            }

            if state.shutdown.is_cancelled() {
                return;
            }
            sample_system_storage(state.system_storage.as_ref(), &state.shutdown).await;
        }
    })
}

async fn sample_system_storage(runtime: &SystemStorageRuntime, shutdown: &CancellationToken) {
    let Ok(permit) = runtime.scan_permit.clone().try_acquire_owned() else {
        return;
    };
    runtime.set_scan_started().await;
    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let roots = runtime.roots.clone();
    let mut worker = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        match roots {
            Ok(roots) => scan_storage_roots(&roots, &worker_cancellation),
            Err(error) => Err(error),
        }
    });

    let result = tokio::select! {
        result = &mut worker => result,
        _ = shutdown.cancelled() => {
            cancellation.cancel();
            worker.await
        }
    };
    let result = match result {
        Ok(result) => result,
        Err(_) => Err(StorageScanFailure::new("worker_failed", false)),
    };
    runtime.publish_result(result).await;
}

fn storage_roots(config: &AppConfig) -> Result<Vec<StorageRoot>, StorageScanFailure> {
    storage_roots_with_maintenance_path(config, config.maintenance_database_path())
}

fn storage_roots_with_maintenance_path(
    config: &AppConfig,
    maintenance_database_path: PathBuf,
) -> Result<Vec<StorageRoot>, StorageScanFailure> {
    let current_dir =
        std::env::current_dir().map_err(|_| StorageScanFailure::new("path_resolution", false))?;
    let absolute = |path: &Path| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            current_dir.join(path)
        }
    };
    let database_path = absolute(&config.database_path);
    let database_parent = database_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| StorageScanFailure::new("path_resolution", false))?;
    let maintenance_database_path = absolute(&maintenance_database_path);

    let mut roots = vec![StorageRoot {
        path: database_parent.to_path_buf(),
        required: true,
    }];
    roots.extend([
        StorageRoot {
            path: absolute(&config.resolved_proxy_raw_dir()),
            required: false,
        },
        StorageRoot {
            path: absolute(&resolved_archive_dir(config)),
            required: false,
        },
        StorageRoot {
            path: absolute(&config.xray_runtime_dir),
            required: false,
        },
    ]);
    for database in [database_path, maintenance_database_path] {
        for path in database_file_and_sidecars(database) {
            roots.push(StorageRoot {
                path,
                required: false,
            });
        }
    }
    Ok(roots)
}

fn database_file_and_sidecars(database: PathBuf) -> [PathBuf; 4] {
    let mut wal = database.as_os_str().to_os_string();
    wal.push("-wal");
    let mut shm = database.as_os_str().to_os_string();
    shm.push("-shm");
    let mut journal = database.as_os_str().to_os_string();
    journal.push("-journal");
    [
        database,
        PathBuf::from(wal),
        PathBuf::from(shm),
        PathBuf::from(journal),
    ]
}

#[derive(Debug, Clone, Copy)]
struct StorageScanFailure {
    reason: &'static str,
    deferred: bool,
}

impl StorageScanFailure {
    fn new(reason: &'static str, deferred: bool) -> Self {
        Self { reason, deferred }
    }

    fn from_io(error: &io::Error) -> Self {
        let reason = match error.kind() {
            io::ErrorKind::PermissionDenied => "permission_denied",
            io::ErrorKind::NotFound => "not_found",
            _ => "io_error",
        };
        Self::new(reason, false)
    }

    fn from_path_io(error: &io::Error) -> Self {
        let reason = match error.kind() {
            io::ErrorKind::PermissionDenied => "permission_denied",
            io::ErrorKind::NotFound => "not_found",
            _ => "path_resolution",
        };
        Self::new(reason, false)
    }
}

#[derive(Debug)]
struct ScanSummary {
    total_bytes: u64,
    visited_objects: usize,
    elapsed: Duration,
}

#[cfg(unix)]
fn scan_storage_roots(
    roots: &[StorageRoot],
    cancellation: &CancellationToken,
) -> Result<ScanSummary, StorageScanFailure> {
    scan_storage_roots_with_limits(
        roots,
        cancellation,
        SYSTEM_STORAGE_RESOURCE_BUDGET_BYTES,
        SYSTEM_STORAGE_MAX_DEPTH,
    )
}

#[cfg(unix)]
fn scan_storage_roots_with_limits(
    roots: &[StorageRoot],
    cancellation: &CancellationToken,
    memory_budget_bytes: usize,
    max_depth: usize,
) -> Result<ScanSummary, StorageScanFailure> {
    scan_storage_roots_with_limits_and_hook(
        roots,
        cancellation,
        memory_budget_bytes,
        max_depth,
        |_| {},
    )
}

#[cfg(unix)]
fn scan_storage_roots_with_limits_and_hook<F>(
    roots: &[StorageRoot],
    cancellation: &CancellationToken,
    memory_budget_bytes: usize,
    max_depth: usize,
    before_walk: F,
) -> Result<ScanSummary, StorageScanFailure>
where
    F: FnMut(&Path),
{
    let started_at = Instant::now();
    let mut scanner = StorageScanner {
        cancellation,
        before_walk,
        seen: HashSet::new(),
        total_bytes: 0,
        processed_since_yield: 0,
        last_yield_at: Instant::now(),
        memory_budget_bytes,
        max_depth,
    };
    for root in roots {
        scanner.check_cancelled()?;
        let root_path = match fs::canonicalize(&root.path) {
            Ok(path) => path,
            Err(error) if !root.required && error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(StorageScanFailure::from_path_io(&error)),
        };
        let path_state_bytes = root_path
            .as_os_str()
            .len()
            .saturating_add(std::mem::size_of::<PathBuf>());
        scanner.ensure_budget(path_state_bytes)?;
        scanner.walk(&root_path, 0, path_state_bytes)?;
    }
    scanner.check_cancelled()?;
    Ok(ScanSummary {
        total_bytes: scanner.total_bytes,
        visited_objects: scanner.seen.len(),
        elapsed: started_at.elapsed(),
    })
}

#[cfg(not(unix))]
fn scan_storage_roots(
    _roots: &[StorageRoot],
    _cancellation: &CancellationToken,
) -> Result<ScanSummary, StorageScanFailure> {
    Err(StorageScanFailure::new("unsupported_platform", false))
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(unix)]
impl FileIdentity {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

#[cfg(unix)]
struct StorageScanner<'a, F> {
    cancellation: &'a CancellationToken,
    before_walk: F,
    seen: HashSet<FileIdentity>,
    total_bytes: u64,
    processed_since_yield: usize,
    last_yield_at: Instant,
    memory_budget_bytes: usize,
    max_depth: usize,
}

#[cfg(unix)]
impl<F> StorageScanner<'_, F>
where
    F: FnMut(&Path),
{
    fn check_cancelled(&self) -> Result<(), StorageScanFailure> {
        if self.cancellation.is_cancelled() {
            Err(StorageScanFailure::new("cancelled", false))
        } else {
            Ok(())
        }
    }

    fn ensure_budget(&self, additional_path_bytes: usize) -> Result<(), StorageScanFailure> {
        let identities = self
            .seen
            .len()
            .checked_mul(IDENTITY_BUDGET_BYTES)
            .ok_or_else(|| StorageScanFailure::new("resource_limit", true))?;
        if identities
            .checked_add(additional_path_bytes)
            .is_none_or(|size| size > self.memory_budget_bytes)
        {
            return Err(StorageScanFailure::new("resource_limit", true));
        }
        Ok(())
    }

    fn insert_identity(
        &mut self,
        identity: FileIdentity,
        allocated_bytes: u64,
        additional_path_bytes: usize,
    ) -> Result<(), StorageScanFailure> {
        if self.seen.contains(&identity) {
            return Ok(());
        }
        let next_identity_count = self
            .seen
            .len()
            .checked_add(1)
            .ok_or_else(|| StorageScanFailure::new("resource_limit", true))?;
        let identity_bytes = next_identity_count
            .checked_mul(IDENTITY_BUDGET_BYTES)
            .ok_or_else(|| StorageScanFailure::new("resource_limit", true))?;
        if identity_bytes
            .checked_add(additional_path_bytes)
            .is_none_or(|size| size > self.memory_budget_bytes)
        {
            return Err(StorageScanFailure::new("resource_limit", true));
        }
        self.seen
            .try_reserve(1)
            .map_err(|_| StorageScanFailure::new("resource_limit", true))?;
        self.seen.insert(identity);
        self.total_bytes = self
            .total_bytes
            .checked_add(allocated_bytes)
            .ok_or_else(|| StorageScanFailure::new("overflow", false))?;
        Ok(())
    }

    fn tick(&mut self) -> Result<(), StorageScanFailure> {
        self.check_cancelled()?;
        self.processed_since_yield += 1;
        if self.processed_since_yield >= SYSTEM_STORAGE_YIELD_ENTRIES
            || self.last_yield_at.elapsed() >= SYSTEM_STORAGE_YIELD_PERIOD
        {
            std::thread::sleep(SYSTEM_STORAGE_YIELD_DURATION);
            self.processed_since_yield = 0;
            self.last_yield_at = Instant::now();
        }
        self.check_cancelled()?;
        Ok(())
    }

    fn walk(
        &mut self,
        path: &Path,
        depth: usize,
        path_state_bytes: usize,
    ) -> Result<(), StorageScanFailure> {
        self.check_cancelled()?;
        (self.before_walk)(path);
        if depth > self.max_depth {
            return Err(StorageScanFailure::new("resource_limit", true));
        }
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(StorageScanFailure::from_io(&error)),
        };
        self.tick()?;
        let identity = FileIdentity::from_metadata(&metadata);
        let allocated_bytes = metadata
            .blocks()
            .checked_mul(512)
            .ok_or_else(|| StorageScanFailure::new("overflow", false))?;

        if metadata.file_type().is_dir() {
            if self.seen.contains(&identity) {
                return Ok(());
            }
            let entries = match fs::read_dir(path) {
                Ok(entries) => entries,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(StorageScanFailure::from_io(&error)),
            };
            self.insert_identity(identity, allocated_bytes, path_state_bytes)?;
            let mut entries = entries;
            loop {
                self.check_cancelled()?;
                let entry = match entries.next() {
                    Some(Ok(entry)) => entry,
                    Some(Err(error)) if error.kind() == io::ErrorKind::NotFound => continue,
                    Some(Err(error)) => return Err(StorageScanFailure::from_io(&error)),
                    None => break,
                };
                let child_name = entry.file_name();
                let child_path_state_bytes = path_state_bytes
                    .checked_add(child_name.as_os_str().len())
                    .and_then(|value| value.checked_add(1))
                    .and_then(|value| value.checked_add(std::mem::size_of::<PathBuf>()))
                    .and_then(|value| value.checked_add(std::mem::size_of::<fs::ReadDir>()))
                    .ok_or_else(|| StorageScanFailure::new("resource_limit", true))?;
                self.ensure_budget(child_path_state_bytes)?;
                let child = path.join(child_name);
                let child_depth = depth + 1;
                self.walk(&child, child_depth, child_path_state_bytes)?;
            }
        } else {
            self.insert_identity(identity, allocated_bytes, path_state_bytes)?;
        }
        Ok(())
    }
}

pub(crate) async fn get_system_storage_snapshot(state: &AppState) -> SystemStorageResponse {
    state.system_storage.snapshot().await
}

#[cfg(all(test, unix))]
pub(crate) fn scan_paths_with_limits_for_test(
    paths: &[PathBuf],
    memory_budget_bytes: usize,
    max_depth: usize,
) -> Result<u64, &'static str> {
    let roots = paths
        .iter()
        .cloned()
        .map(|path| StorageRoot {
            path,
            required: true,
        })
        .collect::<Vec<_>>();
    scan_storage_roots_with_limits(
        &roots,
        &CancellationToken::new(),
        memory_budget_bytes,
        max_depth,
    )
    .map(|summary| summary.total_bytes)
    .map_err(|error| error.reason)
}

#[cfg(all(test, unix))]
pub(crate) fn scan_paths_with_limits_and_hook_for_test<F>(
    paths: &[PathBuf],
    memory_budget_bytes: usize,
    max_depth: usize,
    before_walk: F,
) -> Result<u64, &'static str>
where
    F: FnMut(&Path),
{
    let roots = paths
        .iter()
        .cloned()
        .map(|path| StorageRoot {
            path,
            required: true,
        })
        .collect::<Vec<_>>();
    scan_storage_roots_with_limits_and_hook(
        &roots,
        &CancellationToken::new(),
        memory_budget_bytes,
        max_depth,
        before_walk,
    )
    .map(|summary| summary.total_bytes)
    .map_err(|error| error.reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_DIR_ID: AtomicU64 = AtomicU64::new(0);

    struct TestTempDir(PathBuf);

    impl TestTempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "cvm-system-storage-{}-{}",
                std::process::id(),
                TEMP_DIR_ID.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).expect("create temporary directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestTempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn unsupported_platform_failure_publishes_unknown_without_a_total() {
        let runtime = SystemStorageRuntime::from_roots(Ok(Vec::new()));
        runtime
            .publish_result(Err(StorageScanFailure::new("unsupported_platform", false)))
            .await;

        let snapshot = runtime.snapshot().await;
        assert_eq!(snapshot.state, SystemStorageState::Unknown);
        assert_eq!(snapshot.total_bytes, None);
        assert_eq!(snapshot.reason.as_deref(), Some("unsupported_platform"));
    }

    #[cfg(unix)]
    #[test]
    fn allocated_bytes_deduplicate_hard_links_and_follow_only_configured_roots() {
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::fs::symlink;

        let temp = TestTempDir::new();
        let data = temp.path().join("data");
        let external = temp.path().join("external");
        fs::create_dir_all(&data).expect("data directory");
        fs::create_dir_all(&external).expect("external directory");
        let first = data.join("first.bin");
        fs::write(&first, vec![1_u8; 2049]).expect("file contents");
        fs::hard_link(&first, external.join("hard-link.bin")).expect("hard link");
        fs::write(external.join("copy.bin"), vec![2_u8; 2049]).expect("independent copy");
        symlink(&data, temp.path().join("data-alias")).expect("root symlink");
        symlink(&external, data.join("internal-link")).expect("internal symlink");

        let cancellation = CancellationToken::new();
        let roots = [
            StorageRoot {
                path: data.clone(),
                required: true,
            },
            StorageRoot {
                path: temp.path().join("data-alias"),
                required: false,
            },
            StorageRoot {
                path: external.clone(),
                required: false,
            },
        ];
        let result = scan_storage_roots(&roots, &cancellation).expect("complete scan");
        let allocated = [
            data.clone(),
            first,
            data.join("internal-link"),
            external.clone(),
            external.join("copy.bin"),
        ]
        .into_iter()
        .map(|path| {
            fs::symlink_metadata(path)
                .expect("fixture metadata")
                .blocks()
                * 512
        })
        .sum::<u64>();
        assert_eq!(result.total_bytes, allocated);
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_does_not_publish_partial_scan() {
        let temp = TestTempDir::new();
        fs::write(temp.path().join("payload"), vec![1_u8; 4096]).expect("payload");
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let result = scan_storage_roots(
            &[StorageRoot {
                path: temp.path().to_path_buf(),
                required: true,
            }],
            &cancellation,
        );
        assert_eq!(result.expect_err("cancelled scan").reason, "cancelled");
    }
}
