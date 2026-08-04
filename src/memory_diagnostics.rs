use crate::AppState;
use std::{
    env, fs, io,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
use std::{fs::File, io::Write};
#[cfg(all(target_os = "linux", target_env = "gnu"))]
use std::{fs::OpenOptions, os::fd::IntoRawFd};
use tokio::{task::JoinHandle, time::MissedTickBehavior};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

const MEMORY_SAMPLE_INTERVAL: Duration = Duration::from_secs(30);
const MEMORY_DIAGNOSTICS_MODE_ENV: &str = "MEMORY_DIAGNOSTICS";
const MEMORY_DIAGNOSTICS_PATH_ENV: &str = "MEMORY_DIAGNOSTICS_PATH";
const ALLOCATOR_DIAGNOSTIC_MAX_UNATTRIBUTED_RATIO: u64 = 35;
const ALLOCATOR_DIAGNOSTIC_MAX_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ProcessMemorySnapshot {
    pub(crate) rss_bytes: u64,
    pub(crate) rss_anon_bytes: u64,
    pub(crate) rss_file_bytes: u64,
    pub(crate) swap_bytes: u64,
    pub(crate) peak_rss_bytes: u64,
    pub(crate) threads: u64,
    pub(crate) cgroup_current_bytes: Option<u64>,
    pub(crate) cgroup_limit_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MemoryComponentEstimate {
    pub(crate) entries: usize,
    pub(crate) bytes: usize,
    pub(crate) detail_items: usize,
}

impl MemoryComponentEstimate {
    fn add(self, other: Self) -> Self {
        Self {
            entries: self.entries.saturating_add(other.entries),
            bytes: self.bytes.saturating_add(other.bytes),
            detail_items: self.detail_items.saturating_add(other.detail_items),
        }
    }
}

fn signed_memory_delta(after: u64, before: u64) -> i64 {
    if after >= before {
        after.saturating_sub(before).try_into().unwrap_or(i64::MAX)
    } else {
        before
            .saturating_sub(after)
            .try_into()
            .map(|delta: i64| -delta)
            .unwrap_or(i64::MIN)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct MemoryComponentSnapshot {
    runtime_store: MemoryComponentEstimate,
    terminal_projection: MemoryComponentEstimate,
    dashboard_activity: MemoryComponentEstimate,
    long_term: MemoryComponentEstimate,
    timeseries: MemoryComponentEstimate,
    prompt_cache: MemoryComponentEstimate,
    network_cache: MemoryComponentEstimate,
    routing_cache: MemoryComponentEstimate,
    raw_writer: MemoryComponentEstimate,
    sqlite_writer: MemoryComponentEstimate,
}

impl MemoryComponentSnapshot {
    fn managed_bytes(self) -> usize {
        [
            self.runtime_store,
            self.terminal_projection,
            self.dashboard_activity,
            self.long_term,
            self.timeseries,
            self.prompt_cache,
            self.network_cache,
            self.routing_cache,
            self.raw_writer,
            self.sqlite_writer,
        ]
        .into_iter()
        .fold(
            MemoryComponentEstimate::default(),
            MemoryComponentEstimate::add,
        )
        .bytes
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct MemoryOperationBaseline {
    pub(crate) started_at: Instant,
    pub(crate) process: ProcessMemorySnapshot,
    pub(crate) managed_bytes: usize,
}

#[derive(Debug)]
pub(crate) struct MemoryDiagnosticsRuntime {
    mode: String,
    last_sample: Mutex<Option<ProcessMemorySnapshot>>,
    peak_rss_bytes: AtomicU64,
    high_unattributed_samples: AtomicUsize,
    allocator_once_attempted: AtomicBool,
}

impl Default for MemoryDiagnosticsRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryDiagnosticsRuntime {
    pub(crate) fn new() -> Self {
        Self {
            mode: env::var(MEMORY_DIAGNOSTICS_MODE_ENV)
                .unwrap_or_else(|_| "summary".to_string())
                .trim()
                .to_ascii_lowercase(),
            last_sample: Mutex::new(None),
            peak_rss_bytes: AtomicU64::new(0),
            high_unattributed_samples: AtomicUsize::new(0),
            allocator_once_attempted: AtomicBool::new(false),
        }
    }

    pub(crate) async fn begin_operation(&self, state: &AppState) -> MemoryOperationBaseline {
        MemoryOperationBaseline {
            started_at: Instant::now(),
            process: read_process_memory(),
            managed_bytes: collect_component_snapshot(state).await.managed_bytes(),
        }
    }

    pub(crate) async fn observe_operation(
        &self,
        state: &AppState,
        operation: &'static str,
        baseline: MemoryOperationBaseline,
        load_row_count: u64,
        clone_avoided: bool,
    ) {
        let process = read_process_memory();
        let components = collect_component_snapshot(state).await;
        let managed_bytes = components.managed_bytes();
        info!(
            operation,
            elapsed_ms = baseline.started_at.elapsed().as_millis() as u64,
            retained_bytes = managed_bytes,
            retained_delta_bytes =
                signed_memory_delta(managed_bytes as u64, baseline.managed_bytes as u64),
            peak_delta_bytes = process
                .peak_rss_bytes
                .saturating_sub(baseline.process.peak_rss_bytes),
            rss_delta_bytes = signed_memory_delta(process.rss_bytes, baseline.process.rss_bytes),
            rss_bytes = process.rss_bytes,
            rss_anon_bytes = process.rss_anon_bytes,
            swap_bytes = process.swap_bytes,
            load_row_count,
            clone_avoided,
            pressure_level = memory_pressure_level(process, managed_bytes),
            "memory operation attribution"
        );
    }

    pub(crate) async fn sample(&self, state: &AppState, trigger: &'static str) {
        let process = read_process_memory();
        let components = collect_component_snapshot(state).await;
        let managed_bytes = components.managed_bytes();
        let unattributed_bytes = process.rss_anon_bytes.saturating_sub(managed_bytes as u64);
        self.peak_rss_bytes
            .fetch_max(process.rss_bytes, Ordering::Relaxed);
        if process.rss_anon_bytes > 0
            && unattributed_bytes.saturating_mul(100)
                >= process
                    .rss_anon_bytes
                    .saturating_mul(ALLOCATOR_DIAGNOSTIC_MAX_UNATTRIBUTED_RATIO)
        {
            self.high_unattributed_samples
                .fetch_add(1, Ordering::Relaxed);
        } else {
            self.high_unattributed_samples.store(0, Ordering::Relaxed);
        }

        let sample_delta = self
            .last_sample
            .lock()
            .ok()
            .and_then(|mut previous| previous.replace(process));
        let pressure_level = memory_pressure_level(process, managed_bytes);
        info!(
            trigger,
            rss_bytes = process.rss_bytes,
            rss_anon_bytes = process.rss_anon_bytes,
            rss_file_bytes = process.rss_file_bytes,
            swap_bytes = process.swap_bytes,
            peak_rss_bytes = self.peak_rss_bytes.load(Ordering::Relaxed),
            threads = process.threads,
            cgroup_current_bytes = process.cgroup_current_bytes,
            cgroup_limit_bytes = process.cgroup_limit_bytes,
            managed_bytes,
            unattributed_anon_bytes = unattributed_bytes,
            rss_delta_bytes = sample_delta
                .map(|previous| signed_memory_delta(process.rss_bytes, previous.rss_bytes))
                .unwrap_or_default(),
            runtime_store_bytes = components.runtime_store.bytes,
            terminal_projection_bytes = components.terminal_projection.bytes,
            dashboard_activity_bytes = components.dashboard_activity.bytes,
            long_term_interval_bytes = components.long_term.bytes,
            timeseries_staging_bytes = components.timeseries.bytes,
            prompt_cache_bytes = components.prompt_cache.bytes,
            network_cache_bytes = components.network_cache.bytes,
            routing_cache_bytes = components.routing_cache.bytes,
            raw_writer_bytes = components.raw_writer.bytes,
            sqlite_writer_bytes = components.sqlite_writer.bytes,
            runtime_store_entries = components.runtime_store.entries,
            runtime_record_count = components.runtime_store.detail_items,
            terminal_projection_entries = components.terminal_projection.entries,
            dashboard_activity_entries = components.dashboard_activity.entries,
            long_term_interval_entries = components.long_term.entries,
            timeseries_staging_entries = components.timeseries.entries,
            raw_writer_entries = components.raw_writer.entries,
            pressure_level,
            "memory attribution sample"
        );

        if self.mode == "allocator_once"
            && self.high_unattributed_samples.load(Ordering::Relaxed) >= 3
            && !self.allocator_once_attempted.swap(true, Ordering::Relaxed)
        {
            match capture_allocator_diagnostic() {
                Ok(path) => info!(path = %path.display(), "allocator diagnostic captured"),
                Err(error) => warn!(error = %error, "allocator diagnostic capture failed"),
            }
        }
    }
}

pub(crate) fn spawn_memory_diagnostics(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        state
            .memory_diagnostics
            .sample(state.as_ref(), "startup")
            .await;
        let mut ticker = tokio::time::interval(MEMORY_SAMPLE_INTERVAL);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        ticker.tick().await;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = ticker.tick() => state.memory_diagnostics.sample(state.as_ref(), "periodic").await,
            }
        }
    })
}

async fn collect_component_snapshot(state: &AppState) -> MemoryComponentSnapshot {
    let runtime_store = state.proxy_runtime_invocations.memory_estimate();
    let terminal_health = state.terminal_projection_hub.health();
    let terminal_journal = state.sqlite_batch_writer.terminal_journal_stats();
    let (writer_pending, writer_pending_bytes, _) = state.sqlite_batch_writer.telemetry_snapshot();
    let dashboard_activity = crate::dashboard_activity_snapshot_cache_memory_estimate(
        &state.dashboard_activity_snapshot_cache,
    )
    .await;
    let long_term = state
        .long_term_projection_runtime
        .lock()
        .await
        .memory_estimate();
    let prompt_cache = {
        let cache = state.prompt_cache_conversation_cache.lock().await;
        MemoryComponentEstimate {
            entries: cache.entries.len().saturating_add(cache.in_flight.len()),
            bytes: cache.entries.len().saturating_mul(32 * 1024)
                + cache.in_flight.len().saturating_mul(256),
            detail_items: cache.entries.len(),
        }
    };
    let network_cache = state.dashboard_network_speed_cache.memory_estimate();
    let routing_cache = {
        let selected = state
            .pool_account_selection_runtime
            .selected_at
            .lock()
            .map(|entries| entries.len())
            .unwrap_or_default();
        let runtime = state.pool_routing_runtime_cache.lock().await;
        let runtime_entries = usize::from(runtime.is_some());
        MemoryComponentEstimate {
            entries: selected.saturating_add(runtime_entries),
            bytes: selected.saturating_mul(96) + runtime_entries.saturating_mul(1024),
            detail_items: selected,
        }
    };
    let raw_writer_limit = crate::proxy_raw_async_writer_limit(&state.config);
    let raw_writer_active =
        raw_writer_limit.saturating_sub(state.proxy_raw_async_semaphore.available_permits());
    let raw_writer = MemoryComponentEstimate {
        entries: raw_writer_active,
        // Queue occupancy is tracked at ingress because transport chunks are not size bounded.
        // Keep a small per-writer allowance for encoder/file state that is not queue-owned.
        bytes: crate::proxy_raw_async_writer_queued_bytes()
            .saturating_add(raw_writer_active.saturating_mul(64 * 1024)),
        detail_items: raw_writer_active,
    };

    MemoryComponentSnapshot {
        runtime_store,
        terminal_projection: MemoryComponentEstimate {
            entries: terminal_health.pending_event_count,
            bytes: terminal_health
                .pending_event_bytes
                .saturating_sub(terminal_health.timeseries_pending_bytes)
                .saturating_add(terminal_health.pending_event_count.saturating_mul(128)),
            detail_items: terminal_journal.segment_count,
        },
        dashboard_activity,
        long_term,
        timeseries: MemoryComponentEstimate {
            entries: terminal_health.timeseries_pending_event_count,
            bytes: terminal_health.timeseries_pending_bytes,
            detail_items: terminal_health.timeseries_pending_event_count,
        },
        prompt_cache,
        network_cache,
        routing_cache,
        raw_writer,
        sqlite_writer: MemoryComponentEstimate {
            entries: writer_pending,
            bytes: writer_pending_bytes,
            detail_items: terminal_journal.segment_count,
        },
    }
}

fn memory_pressure_level(process: ProcessMemorySnapshot, managed_bytes: usize) -> &'static str {
    let rss_gib = process.rss_bytes / (1024 * 1024 * 1024);
    let swap_gib = process.swap_bytes / (1024 * 1024 * 1024);
    let cgroup_percent = process
        .cgroup_limit_bytes
        .filter(|limit| *limit > 0)
        .map(|limit| {
            process
                .cgroup_current_bytes
                .unwrap_or_default()
                .saturating_mul(100)
                .checked_div(limit)
                .unwrap_or(100)
        })
        .unwrap_or_default();
    if rss_gib >= 8 || swap_gib >= 8 || cgroup_percent >= 95 {
        "critical"
    } else if rss_gib >= 4
        || swap_gib >= 2
        || managed_bytes >= 1024 * 1024 * 1024
        || cgroup_percent >= 80
    {
        "high"
    } else {
        "normal"
    }
}

fn read_process_memory() -> ProcessMemorySnapshot {
    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
    let smaps = fs::read_to_string("/proc/self/smaps_rollup").unwrap_or_default();
    let rss_bytes = parse_kib(&status, "VmRSS:");
    let rss_anon_bytes =
        parse_optional_kib(&smaps, "Anonymous:").unwrap_or_else(|| parse_kib(&status, "RssAnon:"));
    let rss_file_bytes = parse_kib(&status, "RssFile:");
    let swap_bytes =
        parse_optional_kib(&smaps, "Swap:").unwrap_or_else(|| parse_kib(&status, "VmSwap:"));
    let peak_rss_bytes =
        parse_optional_kib(&status, "VmHWM:").unwrap_or_else(|| parse_kib(&status, "VmPeak:"));
    let threads = parse_value(&status, "Threads:").unwrap_or_default();
    let cgroup_current_bytes = read_cgroup_value(&[
        "/sys/fs/cgroup/memory.current",
        "/sys/fs/cgroup/memory/memory.usage_in_bytes",
    ]);
    let cgroup_limit_bytes = read_cgroup_limit();
    ProcessMemorySnapshot {
        rss_bytes,
        rss_anon_bytes,
        rss_file_bytes,
        swap_bytes,
        peak_rss_bytes,
        threads,
        cgroup_current_bytes,
        cgroup_limit_bytes,
    }
}

fn parse_kib(text: &str, key: &str) -> u64 {
    parse_value(text, key)
        .unwrap_or_default()
        .saturating_mul(1024)
}

fn parse_optional_kib(text: &str, key: &str) -> Option<u64> {
    parse_value(text, key).map(|value| value.saturating_mul(1024))
}

fn parse_value(text: &str, key: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.strip_prefix(key))
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse::<u64>().ok())
}

fn read_cgroup_value(paths: &[&str]) -> Option<u64> {
    paths.iter().find_map(|path| {
        fs::read_to_string(path)
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
    })
}

fn read_cgroup_limit() -> Option<u64> {
    [
        "/sys/fs/cgroup/memory.max",
        "/sys/fs/cgroup/memory/memory.limit_in_bytes",
    ]
    .iter()
    .find_map(|path| {
        let value = fs::read_to_string(path).ok()?;
        let value = value.trim();
        if value == "max" {
            None
        } else {
            value.parse::<u64>().ok()
        }
    })
}

fn capture_allocator_diagnostic() -> io::Result<PathBuf> {
    let path = env::var_os(MEMORY_DIAGNOSTICS_PATH_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(format!(
                "/tmp/codex-vibe-monitor-allocator-{}.xml",
                std::process::id()
            ))
        });
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)?;
        let fd = file.into_raw_fd();
        let mode = std::ffi::CString::new("w").expect("static mode");
        let stream = unsafe { libc::fdopen(fd, mode.as_ptr()) };
        if stream.is_null() {
            unsafe { libc::close(fd) };
            return Err(io::Error::last_os_error());
        }
        let result = unsafe { libc::malloc_info(0, stream) };
        unsafe { libc::fclose(stream) };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        if fs::metadata(&path)?.len() > ALLOCATOR_DIAGNOSTIC_MAX_BYTES {
            fs::remove_file(&path)?;
            return Err(io::Error::other("allocator diagnostic exceeded size limit"));
        }
        Ok(path)
    }
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    {
        let mut file = File::create(&path)?;
        file.write_all(b"allocator diagnostic is unavailable on this target\n")?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_linux_memory_values_without_allocating_business_state() {
        let status = "VmRSS:       2048 kB\nRssAnon:      1024 kB\nThreads:       7\n";
        assert_eq!(parse_kib(status, "VmRSS:"), 2 * 1024 * 1024);
        assert_eq!(parse_kib(status, "RssAnon:"), 1024 * 1024);
        assert_eq!(parse_value(status, "Threads:"), Some(7));
    }

    #[test]
    fn pressure_level_is_soft_and_does_not_cap_or_drop_state() {
        let process = ProcessMemorySnapshot {
            rss_bytes: 5 * 1024 * 1024 * 1024,
            rss_anon_bytes: 5 * 1024 * 1024 * 1024,
            swap_bytes: 2 * 1024 * 1024 * 1024,
            ..Default::default()
        };
        assert_eq!(memory_pressure_level(process, 0), "high");
    }

    #[test]
    fn pressure_level_considers_cgroup_headroom() {
        let process = ProcessMemorySnapshot {
            rss_bytes: 512 * 1024 * 1024,
            rss_anon_bytes: 512 * 1024 * 1024,
            cgroup_current_bytes: Some(850 * 1024 * 1024),
            cgroup_limit_bytes: Some(1024 * 1024 * 1024),
            ..Default::default()
        };
        assert_eq!(memory_pressure_level(process, 0), "high");

        let process = ProcessMemorySnapshot {
            cgroup_current_bytes: Some(980 * 1024 * 1024),
            cgroup_limit_bytes: Some(1024 * 1024 * 1024),
            ..process
        };
        assert_eq!(memory_pressure_level(process, 0), "critical");
    }

    #[test]
    fn signed_memory_delta_preserves_release_direction() {
        assert_eq!(signed_memory_delta(20, 12), 8);
        assert_eq!(signed_memory_delta(12, 20), -8);
        assert_eq!(signed_memory_delta(12, 12), 0);
    }
}
