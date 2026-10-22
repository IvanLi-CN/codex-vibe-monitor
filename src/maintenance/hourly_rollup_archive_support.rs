use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Duration as ChronoDuration, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Asia::Shanghai;
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use once_cell::sync::Lazy;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::{
    AppConfig, ArchiveBatchLayout, ArchiveFileCodec, ArchiveSegmentGranularity,
    INVOCATION_STATUS_WARNING_SUCCESS, format_naive, start_of_local_day,
};

static RETENTION_TEMP_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
const ARCHIVE_FILE_NO_PROGRESS_GRACE: Duration = Duration::from_secs(30);

struct ArchiveFileProgress {
    last_progress_at: Instant,
    processed_bytes: u64,
    total_input_bytes: Option<u64>,
}

impl ArchiveFileProgress {
    fn new(total_input_bytes: Option<u64>) -> Self {
        Self {
            last_progress_at: Instant::now(),
            processed_bytes: 0,
            total_input_bytes,
        }
    }
}

#[derive(Default)]
struct ActiveArchiveFileOperations {
    writers: HashSet<PathBuf>,
    readers: HashMap<PathBuf, usize>,
}

static ACTIVE_ARCHIVE_FILE_OPERATIONS: Lazy<std::sync::Mutex<ActiveArchiveFileOperations>> =
    Lazy::new(|| std::sync::Mutex::new(ActiveArchiveFileOperations::default()));

pub(crate) struct ArchiveFileOperationLease {
    paths: Vec<PathBuf>,
    shared_read_paths: Vec<PathBuf>,
    timed_out: Arc<AtomicBool>,
}

struct ArchiveFileOperationCancellationSignal(Arc<AtomicBool>);

impl Drop for ArchiveFileOperationCancellationSignal {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

impl ArchiveFileOperationLease {
    fn register(paths: impl IntoIterator<Item = PathBuf>) -> Option<Self> {
        Self::register_with_shared_reads(paths, [])
    }

    fn register_with_shared_reads(
        paths: impl IntoIterator<Item = PathBuf>,
        shared_read_paths: impl IntoIterator<Item = PathBuf>,
    ) -> Option<Self> {
        let paths = paths
            .into_iter()
            .map(|path| archive_file_operation_key(&path))
            .collect::<HashSet<_>>();
        let shared_read_paths = shared_read_paths
            .into_iter()
            .map(|path| archive_file_operation_key(&path))
            .collect::<HashSet<_>>();
        if paths.is_empty() && shared_read_paths.is_empty() {
            return None;
        }
        let mut active = ACTIVE_ARCHIVE_FILE_OPERATIONS.lock().ok()?;
        if paths
            .iter()
            .any(|path| active.writers.contains(path) || active.readers.contains_key(path))
            || shared_read_paths
                .iter()
                .any(|path| active.writers.contains(path))
            || paths.iter().any(|path| shared_read_paths.contains(path))
        {
            return None;
        }
        active.writers.extend(paths.iter().cloned());
        for path in &shared_read_paths {
            *active.readers.entry(path.clone()).or_default() += 1;
        }
        Some(Self {
            paths: paths.into_iter().collect(),
            shared_read_paths: shared_read_paths.into_iter().collect(),
            timed_out: Arc::new(AtomicBool::new(false)),
        })
    }
}

impl Drop for ArchiveFileOperationLease {
    fn drop(&mut self) {
        if let Ok(mut active) = ACTIVE_ARCHIVE_FILE_OPERATIONS.lock() {
            for path in &self.paths {
                active.writers.remove(path);
            }
            for path in &self.shared_read_paths {
                if let Some(readers) = active.readers.get_mut(path) {
                    if *readers <= 1 {
                        active.readers.remove(path);
                    } else {
                        *readers -= 1;
                    }
                }
            }
        }
    }
}

fn archive_file_operation_key(path: &Path) -> PathBuf {
    let path_string = path.to_string_lossy();
    for suffix in [".source-meta", "-journal", "-wal", "-shm"] {
        if let Some(base) = path_string.strip_suffix(suffix) {
            return PathBuf::from(base);
        }
    }
    if path.extension().and_then(|extension| extension.to_str()) == Some("owner") {
        return path.with_extension("sqlite");
    }
    path.to_path_buf()
}

pub(crate) fn try_acquire_archive_file_operation_lease(
    path: &Path,
) -> Option<ArchiveFileOperationLease> {
    ArchiveFileOperationLease::register([path.to_path_buf()])
}

pub(crate) fn is_archive_file_operation_active(path: &Path) -> bool {
    ACTIVE_ARCHIVE_FILE_OPERATIONS
        .lock()
        .map(|active| {
            let key = archive_file_operation_key(path);
            active.writers.contains(&key) || active.readers.contains_key(&key)
        })
        .unwrap_or(true)
}

pub(crate) fn resolved_raw_path_candidates(
    path: &str,
    fallback_root: Option<&Path>,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let primary = PathBuf::from(path);
    candidates.push(normalize_path_for_compare(&primary));
    if !primary.is_absolute()
        && let Some(root) = fallback_root
    {
        let fallback = root.join(&primary);
        let normalized = normalize_path_for_compare(&fallback);
        if !candidates.contains(&normalized) {
            candidates.push(normalized);
        }
    }
    candidates
}

pub(crate) fn resolved_raw_path_read_candidates(
    path: &str,
    fallback_root: Option<&Path>,
) -> Vec<PathBuf> {
    let mut candidates = resolved_raw_path_candidates(path, fallback_root);
    if let Some(alternate_path) = raw_payload_alternate_db_path(path) {
        for candidate in resolved_raw_path_candidates(&alternate_path, fallback_root) {
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
    }
    candidates
}

pub(crate) fn raw_payload_alternate_db_path(path: &str) -> Option<String> {
    if path.ends_with(".bin.gz") {
        Some(path.trim_end_matches(".gz").to_string())
    } else if path.ends_with(".bin") {
        Some(format!("{path}.gz"))
    } else {
        None
    }
}

pub(crate) fn normalize_path_for_compare(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

pub(crate) fn count_planned_proxy_raw_paths(
    raw_paths: &[Option<String>],
    raw_path_fallback_root: Option<&Path>,
) -> usize {
    let mut seen = HashSet::new();
    for raw_path in raw_paths.iter().flatten() {
        for candidate in resolved_raw_path_read_candidates(raw_path, raw_path_fallback_root) {
            if candidate.exists() {
                seen.insert(candidate);
            }
        }
    }
    seen.len()
}

pub(crate) fn delete_proxy_raw_paths(
    plan: super::retention::ProxyRawDeletionPlan,
    raw_path_fallback_root: Option<&Path>,
) -> Result<usize> {
    let super::retention::ProxyRawDeletionPlan {
        paths: raw_paths,
        fences,
        identities,
    } = plan;
    // Keep every exclusive raw-directory fence until all identity checks and unlinks finish.
    let _fences = fences;
    let mut removed = 0usize;
    let mut seen = HashSet::new();
    for raw_path in raw_paths.iter().flatten() {
        for candidate in resolved_raw_path_read_candidates(raw_path, raw_path_fallback_root) {
            if !seen.insert(candidate.clone()) {
                continue;
            }
            let Some(expected) = identities.iter().find(|entry| entry.path == candidate) else {
                continue;
            };
            let metadata = match fs::metadata(&candidate) {
                Ok(metadata) if metadata.file_type().is_file() => metadata,
                Ok(_) | Err(_) => continue,
            };
            if retention_raw_file_identity(&metadata) != expected.identity
                || metadata.len() != expected.byte_size
            {
                warn!(
                    path = %candidate.display(),
                    "retaining raw payload because its identity changed before deletion"
                );
                continue;
            }
            match fs::remove_file(&candidate) {
                Ok(_) => {
                    removed += 1;
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => {
                    warn!(path = %candidate.display(), error = %err, "failed to remove raw payload file");
                }
            }
        }
    }
    Ok(removed)
}

pub(crate) fn shanghai_retention_cutoff(days: u64) -> DateTime<Utc> {
    start_of_local_day(Utc::now(), Shanghai) - ChronoDuration::days(days as i64)
}

pub(crate) fn shanghai_local_cutoff_string(days: u64) -> String {
    format_naive(
        shanghai_retention_cutoff(days)
            .with_timezone(&Shanghai)
            .naive_local(),
    )
}

pub(crate) fn shanghai_local_cutoff_for_age_secs_string(age_secs: u64) -> String {
    format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local()
            - ChronoDuration::seconds(age_secs as i64),
    )
}

pub(crate) fn shanghai_utc_cutoff_string(days: u64) -> String {
    format_naive(shanghai_retention_cutoff(days).naive_utc())
}

pub(crate) fn invocation_status_is_success_like(
    status: Option<&str>,
    error_message: Option<&str>,
) -> bool {
    let normalized_status = status.map(str::trim).unwrap_or_default();
    let error_message_empty = error_message.map(str::trim).is_none_or(str::is_empty);

    normalized_status.eq_ignore_ascii_case("success")
        || normalized_status.eq_ignore_ascii_case("completed")
        || normalized_status.eq_ignore_ascii_case(INVOCATION_STATUS_WARNING_SUCCESS)
        || (normalized_status.eq_ignore_ascii_case("http_200") && error_message_empty)
}

pub(crate) fn invocation_status_is_success_like_sql(
    status_column: &str,
    error_message_column: &str,
) -> String {
    format!(
        "(LOWER(TRIM(COALESCE({status_column}, ''))) IN ('success', 'completed', '{warning_success}') OR (LOWER(TRIM(COALESCE({status_column}, ''))) = 'http_200' AND TRIM(COALESCE({error_message_column}, '')) = ''))",
        warning_success = INVOCATION_STATUS_WARNING_SUCCESS,
    )
}

pub(crate) fn parse_shanghai_local_naive(value: &str) -> Result<NaiveDateTime> {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .with_context(|| format!("invalid shanghai-local timestamp: {value}"))
}

pub(crate) fn parse_utc_naive(value: &str) -> Result<NaiveDateTime> {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .with_context(|| format!("invalid utc timestamp: {value}"))
}

pub(crate) fn shanghai_day_key_from_local_naive(value: &str) -> Result<String> {
    Ok(parse_shanghai_local_naive(value)?
        .format("%Y-%m-%d")
        .to_string())
}

pub(crate) fn shanghai_month_key_from_local_naive(value: &str) -> Result<String> {
    Ok(parse_shanghai_local_naive(value)?
        .format("%Y-%m")
        .to_string())
}

pub(crate) fn shanghai_month_key_from_utc_naive(value: &str) -> Result<String> {
    let utc = Utc.from_utc_datetime(&parse_utc_naive(value)?);
    Ok(utc.with_timezone(&Shanghai).format("%Y-%m").to_string())
}

pub(crate) fn resolved_archive_dir(config: &AppConfig) -> PathBuf {
    resolve_path_from_database_parent(&config.database_path, &config.archive_dir)
}

pub(crate) fn resolve_path_from_database_parent(
    database_path: &Path,
    configured_path: &Path,
) -> PathBuf {
    if configured_path.is_absolute() {
        return configured_path.to_path_buf();
    }

    match database_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(configured_path),
        _ => configured_path.to_path_buf(),
    }
}

pub(crate) fn archive_batch_file_path(
    config: &AppConfig,
    dataset: &str,
    month_key: &str,
) -> Result<PathBuf> {
    let year = month_key
        .split('-')
        .next()
        .filter(|segment| segment.len() == 4)
        .ok_or_else(|| anyhow!("invalid month key: {month_key}"))?;
    Ok(resolved_archive_dir(config)
        .join(dataset)
        .join(year)
        .join(format!("{dataset}-{month_key}.sqlite.gz")))
}

pub(crate) fn archive_segment_file_path(
    config: &AppConfig,
    dataset: &str,
    day_key: &str,
    part_key: &str,
    codec: ArchiveFileCodec,
) -> Result<PathBuf> {
    let mut segments = day_key.split('-');
    let year = segments
        .next()
        .filter(|segment| segment.len() == 4)
        .ok_or_else(|| anyhow!("invalid day key: {day_key}"))?;
    let month = segments
        .next()
        .filter(|segment| segment.len() == 2)
        .ok_or_else(|| anyhow!("invalid day key: {day_key}"))?;
    let day = segments
        .next()
        .filter(|segment| segment.len() == 2)
        .ok_or_else(|| anyhow!("invalid day key: {day_key}"))?;
    Ok(resolved_archive_dir(config)
        .join(dataset)
        .join(year)
        .join(month)
        .join(day)
        .join(format!("{part_key}.sqlite.{}", codec.file_extension())))
}

pub(crate) fn archive_month_key_from_day_key(day_key: &str) -> Result<String> {
    Ok(day_key
        .get(..7)
        .ok_or_else(|| anyhow!("invalid day key: {day_key}"))?
        .to_string())
}

pub(crate) fn retention_temp_suffix() -> String {
    format!(
        "{}-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        RETENTION_TEMP_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

pub(crate) fn archive_layout_for_dataset(config: &AppConfig, dataset: &str) -> ArchiveBatchLayout {
    if dataset == "codex_invocations" {
        config.codex_invocation_archive_layout
    } else {
        ArchiveBatchLayout::LegacyMonth
    }
}

pub(crate) fn invocation_archive_group_key(
    config: &AppConfig,
    occurred_at: &str,
) -> Result<String> {
    match config.codex_invocation_archive_layout {
        ArchiveBatchLayout::LegacyMonth => shanghai_month_key_from_local_naive(occurred_at),
        ArchiveBatchLayout::SegmentV1 => {
            match config.codex_invocation_archive_segment_granularity {
                ArchiveSegmentGranularity::Day => shanghai_day_key_from_local_naive(occurred_at),
            }
        }
    }
}

pub(crate) fn archive_segment_part_key_for_ids(ids: &[i64]) -> Result<String> {
    if ids.is_empty() {
        return Err(anyhow!("archive segment requires at least one row id"));
    }
    let mut normalized_ids = ids.to_vec();
    normalized_ids.sort_unstable();
    normalized_ids.dedup();
    let mut hasher = Sha256::new();
    for id in &normalized_ids {
        hasher.update(id.to_be_bytes());
    }
    let digest = format!("{:x}", hasher.finalize());
    Ok(format!(
        "part-{:016x}-{:016x}-{}",
        normalized_ids.first().expect("non-empty ids"),
        normalized_ids.last().expect("non-empty ids"),
        &digest[..16]
    ))
}

pub(crate) fn is_archive_temp_file_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    (lower.contains(".sqlite.gz.") || lower.contains(".sqlite.zst."))
        && (lower.ends_with(".sqlite")
            || lower.ends_with(".tmp")
            || lower.ends_with(".partial")
            || lower.ends_with(".owner")
            || lower.ends_with(".source-meta")
            || lower.ends_with(".sqlite-journal")
            || lower.ends_with(".sqlite-wal")
            || lower.ends_with(".sqlite-shm"))
}

pub(crate) fn collect_archive_file_paths(root: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)
        .with_context(|| format!("failed to read archive directory {}", root.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_archive_file_paths(&path, files)?;
        } else if file_type.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

pub(crate) fn inflate_gzip_sqlite_file(source: &Path, destination: &Path) -> Result<()> {
    inflate_gzip_sqlite_file_with_limits(source, destination, None, None, None, None)
}

pub(crate) async fn inflate_gzip_sqlite_file_with_operation_budget(
    source: &Path,
    destination: &Path,
) -> Result<()> {
    let source = source.to_path_buf();
    let destination = destination.to_path_buf();
    run_archive_file_work_with_operation_budget(
        "inflate",
        vec![destination.clone()],
        vec![source.clone()],
        vec![destination.clone()],
        vec![destination.clone()],
        move |deadline, shutdown, cancellation, progress| {
            inflate_gzip_sqlite_file_with_limits(
                &source,
                &destination,
                deadline,
                shutdown.as_ref(),
                Some(cancellation.as_ref()),
                Some(progress.as_ref()),
            )
        },
    )
    .await
}

pub(crate) async fn inflate_gzip_sqlite_file_with_elapsed_budget(
    source: &Path,
    destination: &Path,
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> Result<bool> {
    let source = source.to_path_buf();
    let destination = destination.to_path_buf();
    let deadline = max_elapsed.map(|limit| started_at + limit);
    let timeout = deadline
        .map(|deadline| deadline.saturating_duration_since(Instant::now()))
        .filter(|remaining| !remaining.is_zero());
    run_archive_file_work_with_deadline(
        ArchiveFileWorkRequest {
            operation: "historical_rollup_archive_inflate",
            protected_paths: vec![destination.clone()],
            shared_read_paths: vec![source.clone()],
            failure_cleanup_paths: vec![destination.clone()],
            timeout_cleanup_paths: vec![destination.clone()],
            deadline,
            timeout,
            shutdown: super::retention::retention_run_shutdown_token(),
        },
        move |operation_deadline, shutdown, cancellation, progress| {
            inflate_gzip_sqlite_file_with_elapsed_limits(
                &source,
                &destination,
                operation_deadline,
                shutdown.as_ref(),
                Some(cancellation.as_ref()),
                Some(progress.as_ref()),
            )
        },
    )
    .await
}

fn inflate_gzip_sqlite_file_with_limits(
    source: &Path,
    destination: &Path,
    operation_deadline: Option<Instant>,
    shutdown: Option<&CancellationToken>,
    cancellation: Option<&AtomicBool>,
    progress: Option<&Mutex<ArchiveFileProgress>>,
) -> Result<()> {
    let input = fs::File::open(source)
        .with_context(|| format!("failed to open archive batch {}", source.display()))?;
    let mut decoder = GzDecoder::new(input);
    let output = fs::File::create(destination)
        .with_context(|| format!("failed to create temp archive db {}", destination.display()))?;
    let total_input_bytes = fs::metadata(source).ok().map(|metadata| metadata.len());
    let mut writer = io::BufWriter::new(output);
    copy_archive_bytes_with_limits(
        &mut decoder,
        &mut writer,
        ArchiveCopyLimits {
            destination,
            operation_deadline,
            shutdown,
            cancellation,
            total_input_bytes,
            progress,
        },
    )
    .with_context(|| {
        format!(
            "failed to decompress archive batch {} into {}",
            source.display(),
            destination.display()
        )
    })?;
    writer.flush()?;
    Ok(())
}

fn inflate_gzip_sqlite_file_with_elapsed_limits(
    source: &Path,
    destination: &Path,
    operation_deadline: Option<Instant>,
    shutdown: Option<&CancellationToken>,
    cancellation: Option<&AtomicBool>,
    progress: Option<&Mutex<ArchiveFileProgress>>,
) -> Result<bool> {
    let input = fs::File::open(source)
        .with_context(|| format!("failed to open archive batch {}", source.display()))?;
    let mut decoder = GzDecoder::new(input);
    let output = fs::File::create(destination)
        .with_context(|| format!("failed to create temp archive db {}", destination.display()))?;
    let total_input_bytes = fs::metadata(source).ok().map(|metadata| metadata.len());
    let mut writer = io::BufWriter::new(output);
    let completed = copy_archive_bytes_with_elapsed_budget(
        &mut decoder,
        &mut writer,
        operation_deadline,
        shutdown,
        cancellation,
        total_input_bytes,
        progress,
    )
    .with_context(|| {
        format!(
            "failed to decompress archive batch {} into {}",
            source.display(),
            destination.display()
        )
    })?;
    writer.flush()?;
    Ok(completed)
}

pub(crate) fn deflate_sqlite_file_to_gzip(source: &Path, destination: &Path) -> Result<()> {
    deflate_sqlite_file_to_gzip_with_limits(source, destination, None, None, None, None)
}

pub(crate) async fn deflate_sqlite_file_to_gzip_with_operation_budget(
    source: &Path,
    destination: &Path,
) -> Result<()> {
    let source = source.to_path_buf();
    let destination = destination.to_path_buf();
    run_archive_file_work_with_operation_budget(
        "deflate",
        vec![source.clone(), destination.clone()],
        Vec::new(),
        vec![destination.clone()],
        vec![source.clone(), destination.clone()],
        move |deadline, shutdown, cancellation, progress| {
            deflate_sqlite_file_to_gzip_with_limits(
                &source,
                &destination,
                deadline,
                shutdown.as_ref(),
                Some(cancellation.as_ref()),
                Some(progress.as_ref()),
            )
        },
    )
    .await
}

fn deflate_sqlite_file_to_gzip_with_limits(
    source: &Path,
    destination: &Path,
    operation_deadline: Option<Instant>,
    shutdown: Option<&CancellationToken>,
    cancellation: Option<&AtomicBool>,
    progress: Option<&Mutex<ArchiveFileProgress>>,
) -> Result<()> {
    let input = fs::File::open(source)
        .with_context(|| format!("failed to open temp archive db {}", source.display()))?;
    let output = fs::File::create(destination)
        .with_context(|| format!("failed to create archive gzip {}", destination.display()))?;
    let mut encoder = GzEncoder::new(io::BufWriter::new(output), Compression::default());
    let mut reader = io::BufReader::new(input);
    let total_input_bytes = fs::metadata(source).ok().map(|metadata| metadata.len());
    copy_archive_bytes_with_limits(
        &mut reader,
        &mut encoder,
        ArchiveCopyLimits {
            destination,
            operation_deadline,
            shutdown,
            cancellation,
            total_input_bytes,
            progress,
        },
    )
    .with_context(|| {
        format!(
            "failed to compress temp archive db {} into {}",
            source.display(),
            destination.display()
        )
    })?;
    let mut writer = encoder
        .finish()
        .context("failed to finish archive gzip writer")?;
    writer.flush()?;
    Ok(())
}

async fn run_archive_file_work_with_operation_budget<T, F>(
    operation: &'static str,
    protected_paths: Vec<PathBuf>,
    shared_read_paths: Vec<PathBuf>,
    failure_cleanup_paths: Vec<PathBuf>,
    timeout_cleanup_paths: Vec<PathBuf>,
    work: F,
) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(
            Option<Instant>,
            Option<CancellationToken>,
            Arc<AtomicBool>,
            Arc<Mutex<ArchiveFileProgress>>,
        ) -> Result<T>
        + Send
        + 'static,
{
    let shutdown = super::retention::retention_run_shutdown_token();
    // Production compression is governed by cancellation and the no-progress guard. The
    // legacy fixed operation budget remains available only to deterministic unit tests.
    #[cfg(test)]
    let (deadline, timeout) = {
        let deadline = super::retention::retention_archive_file_operation_deadline();
        (
            Some(deadline),
            Some(deadline.saturating_duration_since(Instant::now())),
        )
    };
    #[cfg(not(test))]
    let (deadline, timeout) = (None, None);
    run_archive_file_work_with_deadline(
        ArchiveFileWorkRequest {
            operation,
            protected_paths,
            shared_read_paths,
            failure_cleanup_paths,
            timeout_cleanup_paths,
            deadline,
            timeout,
            shutdown,
        },
        work,
    )
    .await
}

struct ArchiveFileWorkRequest {
    operation: &'static str,
    protected_paths: Vec<PathBuf>,
    shared_read_paths: Vec<PathBuf>,
    failure_cleanup_paths: Vec<PathBuf>,
    timeout_cleanup_paths: Vec<PathBuf>,
    deadline: Option<Instant>,
    timeout: Option<Duration>,
    shutdown: Option<CancellationToken>,
}

async fn run_archive_file_work_with_deadline<T, F>(
    request: ArchiveFileWorkRequest,
    work: F,
) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(
            Option<Instant>,
            Option<CancellationToken>,
            Arc<AtomicBool>,
            Arc<Mutex<ArchiveFileProgress>>,
        ) -> Result<T>
        + Send
        + 'static,
{
    let ArchiveFileWorkRequest {
        operation,
        protected_paths,
        shared_read_paths,
        failure_cleanup_paths,
        timeout_cleanup_paths,
        deadline,
        timeout,
        shutdown,
    } = request;
    let Some(lease) =
        ArchiveFileOperationLease::register_with_shared_reads(protected_paths, shared_read_paths)
    else {
        return Err(super::retention::retention_write_deferred(operation));
    };
    let timed_out = Arc::clone(&lease.timed_out);
    let worker_timed_out = Arc::clone(&timed_out);
    let worker_cancellation = Arc::clone(&timed_out);
    let progress = Arc::new(Mutex::new(ArchiveFileProgress::new(None)));
    let worker_progress = Arc::clone(&progress);
    let _cancellation_signal = ArchiveFileOperationCancellationSignal(Arc::clone(&timed_out));
    let mut worker = tokio::task::spawn_blocking(move || {
        let _lease = lease;
        let cleanup_paths = |paths: &[PathBuf]| {
            for path in paths {
                super::retention::remove_temp_sqlite_artifacts_unchecked(path);
            }
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            work(deadline, shutdown, worker_cancellation, worker_progress)
        }));
        match result {
            Ok(result) => {
                if result.is_err() {
                    cleanup_paths(&failure_cleanup_paths);
                }
                if worker_timed_out.load(Ordering::Acquire) {
                    for path in &timeout_cleanup_paths {
                        super::retention::remove_temp_archive_file_operation_artifacts_unchecked(
                            path,
                        );
                    }
                }
                result
            }
            Err(payload) => {
                cleanup_paths(&failure_cleanup_paths);
                if worker_timed_out.load(Ordering::Acquire) {
                    for path in &timeout_cleanup_paths {
                        super::retention::remove_temp_archive_file_operation_artifacts_unchecked(
                            path,
                        );
                    }
                }
                std::panic::resume_unwind(payload);
            }
        }
    });
    let watchdog_triggered = Arc::new(AtomicBool::new(false));
    let watchdog = (deadline.is_none() && timeout.is_none()).then(|| {
        let progress = Arc::clone(&progress);
        let timed_out = Arc::clone(&timed_out);
        let watchdog_triggered = Arc::clone(&watchdog_triggered);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(ARCHIVE_FILE_NO_PROGRESS_GRACE).await;
                let stale = progress
                    .lock()
                    .map(|state| state.last_progress_at.elapsed() >= ARCHIVE_FILE_NO_PROGRESS_GRACE)
                    .unwrap_or(true);
                if stale {
                    watchdog_triggered.store(true, Ordering::Release);
                    timed_out.store(true, Ordering::Release);
                    return;
                }
            }
        })
    });
    let joined = match timeout {
        Some(timeout) => match tokio::time::timeout(timeout, &mut worker).await {
            Ok(joined) => joined,
            Err(_) => {
                timed_out.store(true, Ordering::Release);
                match worker.await {
                    Ok(_) => {}
                    Err(error) => {
                        return Err(anyhow!(
                            "retention archive {operation} worker panicked after operation budget: {error}"
                        ));
                    }
                }
                return Err(super::retention::retention_archive_file_operation_budget_exhausted());
            }
        },
        None => match watchdog {
            Some(mut watchdog) => {
                tokio::select! {
                    joined = &mut worker => {
                        watchdog.abort();
                        let joined = joined.with_context(|| {
                            format!("retention archive {operation} worker panicked")
                        })?;
                        if watchdog_triggered.load(Ordering::Acquire) {
                            let (processed_bytes, total_input_bytes) = progress
                                .lock()
                                .map(|state| (state.processed_bytes, state.total_input_bytes))
                                .unwrap_or((0, None));
                            return Err(
                                super::retention::retention_archive_file_operation_no_progress(
                                    processed_bytes,
                                    total_input_bytes,
                                ),
                            );
                        }
                        return joined;
                    }
                    _ = &mut watchdog => {
                        match worker.await {
                            Ok(_) => {}
                            Err(error) => {
                                return Err(anyhow!(
                                    "retention archive {operation} worker panicked after no-progress watchdog: {error}"
                                ));
                            }
                        }
                        let (processed_bytes, total_input_bytes) = progress
                            .lock()
                            .map(|state| (state.processed_bytes, state.total_input_bytes))
                            .unwrap_or((0, None));
                        return Err(
                            super::retention::retention_archive_file_operation_no_progress(
                                processed_bytes,
                                total_input_bytes,
                            ),
                        );
                    }
                }
            }
            None => worker.await,
        },
    };
    joined.with_context(|| format!("retention archive {operation} worker panicked"))?
}

struct ArchiveCopyLimits<'a> {
    destination: &'a Path,
    operation_deadline: Option<Instant>,
    shutdown: Option<&'a CancellationToken>,
    cancellation: Option<&'a AtomicBool>,
    total_input_bytes: Option<u64>,
    progress: Option<&'a Mutex<ArchiveFileProgress>>,
}

fn copy_archive_bytes_with_limits(
    reader: &mut impl io::Read,
    writer: &mut impl io::Write,
    limits: ArchiveCopyLimits<'_>,
) -> Result<()> {
    let ArchiveCopyLimits {
        destination,
        operation_deadline,
        shutdown,
        cancellation,
        total_input_bytes,
        progress,
    } = limits;
    let mut buffer = [0_u8; 64 * 1024];
    let mut processed_bytes = 0_u64;
    let mut last_progress_at = Instant::now();
    if let Some(progress) = progress
        && let Ok(mut state) = progress.lock()
    {
        state.total_input_bytes = total_input_bytes;
        state.last_progress_at = last_progress_at;
    }
    loop {
        if operation_deadline.is_some_and(|deadline| Instant::now() >= deadline)
            || (operation_deadline.is_none() && super::retention::retention_run_budget_expired())
            || cancellation.is_some_and(|flag| flag.load(Ordering::Acquire))
        {
            return Err(super::retention::retention_archive_file_operation_budget_exhausted());
        }
        if operation_deadline.is_none()
            && last_progress_at.elapsed() >= ARCHIVE_FILE_NO_PROGRESS_GRACE
        {
            return Err(
                super::retention::retention_archive_file_operation_no_progress(
                    processed_bytes,
                    total_input_bytes,
                ),
            );
        }
        if shutdown.is_some_and(CancellationToken::is_cancelled)
            || (shutdown.is_none()
                && super::retention::retention_run_shutdown_token()
                    .is_some_and(|shutdown| shutdown.is_cancelled()))
        {
            return Err(super::retention::retention_write_deferred(
                "archive_file_work",
            ));
        }
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(());
        }
        if crate::filesystem_available_bytes(destination)
            .is_some_and(|available| available < 64 * 1024 * 1024 + read as u64)
        {
            bail!("insufficient disk space for task-local archive file work");
        }
        writer.write_all(&buffer[..read])?;
        processed_bytes = processed_bytes.saturating_add(read as u64);
        last_progress_at = Instant::now();
        if let Some(progress) = progress
            && let Ok(mut state) = progress.lock()
        {
            state.processed_bytes = processed_bytes;
            state.last_progress_at = last_progress_at;
        }
    }
}

fn copy_archive_bytes_with_elapsed_budget(
    reader: &mut impl io::Read,
    writer: &mut impl io::Write,
    operation_deadline: Option<Instant>,
    shutdown: Option<&CancellationToken>,
    cancellation: Option<&AtomicBool>,
    total_input_bytes: Option<u64>,
    progress: Option<&Mutex<ArchiveFileProgress>>,
) -> Result<bool> {
    let mut buffer = [0_u8; 64 * 1024];
    let mut processed_bytes = 0_u64;
    if let Some(progress) = progress
        && let Ok(mut state) = progress.lock()
    {
        state.total_input_bytes = total_input_bytes;
        state.last_progress_at = Instant::now();
    }
    loop {
        if shutdown.is_some_and(CancellationToken::is_cancelled)
            || cancellation.is_some_and(|flag| flag.load(Ordering::Acquire))
        {
            return Err(super::retention::retention_write_deferred(
                "historical_rollup_archive_inflate",
            ));
        }
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(true);
        }
        writer.write_all(&buffer[..read])?;
        if shutdown.is_some_and(CancellationToken::is_cancelled)
            || cancellation.is_some_and(|flag| flag.load(Ordering::Acquire))
        {
            return Err(super::retention::retention_write_deferred(
                "historical_rollup_archive_inflate",
            ));
        }
        processed_bytes = processed_bytes.saturating_add(read as u64);
        if let Some(progress) = progress
            && let Ok(mut state) = progress.lock()
        {
            state.processed_bytes = processed_bytes;
            state.last_progress_at = Instant::now();
        }
        if operation_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Ok(false);
        }
    }
}

pub(crate) fn sha256_hex_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to open file for sha256 {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 8192];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub(crate) fn sha256_hex_file_with_deadline(path: &Path, deadline: Instant) -> Result<String> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to open file for sha256 {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 8192];
    loop {
        if Instant::now() >= deadline {
            return Err(super::retention::retention_archive_file_operation_budget_exhausted());
        }
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub(crate) async fn sha256_hex_file_with_operation_budget(path: &Path) -> Result<String> {
    let path = path.to_path_buf();
    run_archive_file_work_with_operation_budget(
        "archive_file_hash",
        Vec::new(),
        vec![path.clone()],
        Vec::new(),
        Vec::new(),
        move |deadline, shutdown, cancellation, progress| {
            sha256_hex_file_with_progress_guard(
                &path,
                deadline,
                shutdown.as_ref(),
                Some(cancellation.as_ref()),
                Some(progress.as_ref()),
            )
        },
    )
    .await
}

fn sha256_hex_file_with_progress_guard(
    path: &Path,
    operation_deadline: Option<Instant>,
    shutdown: Option<&CancellationToken>,
    cancellation: Option<&AtomicBool>,
    progress: Option<&Mutex<ArchiveFileProgress>>,
) -> Result<String> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to open file for sha256 {}", path.display()))?;
    let total_input_bytes = file.metadata().ok().map(|metadata| metadata.len());
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 8192];
    let mut processed_bytes = 0_u64;
    let mut last_progress_at = Instant::now();
    if let Some(progress) = progress
        && let Ok(mut state) = progress.lock()
    {
        state.total_input_bytes = total_input_bytes;
        state.last_progress_at = last_progress_at;
    }
    loop {
        if operation_deadline.is_some_and(|deadline| Instant::now() >= deadline)
            || (operation_deadline.is_none() && super::retention::retention_run_budget_expired())
            || cancellation.is_some_and(|flag| flag.load(Ordering::Acquire))
        {
            return Err(super::retention::retention_archive_file_operation_budget_exhausted());
        }
        if shutdown.is_some_and(CancellationToken::is_cancelled)
            || (shutdown.is_none()
                && super::retention::retention_run_shutdown_token()
                    .is_some_and(|shutdown| shutdown.is_cancelled()))
        {
            return Err(super::retention::retention_write_deferred(
                "archive_file_hash",
            ));
        }
        if last_progress_at.elapsed() >= ARCHIVE_FILE_NO_PROGRESS_GRACE {
            return Err(
                super::retention::retention_archive_file_operation_no_progress(
                    processed_bytes,
                    total_input_bytes,
                ),
            );
        }
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
        processed_bytes = processed_bytes.saturating_add(read as u64);
        last_progress_at = Instant::now();
        if let Some(progress) = progress
            && let Ok(mut state) = progress.lock()
        {
            state.processed_bytes = processed_bytes;
            state.last_progress_at = last_progress_at;
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_file_operation_lease_fences_all_paths() {
        let suffix = RETENTION_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let source = PathBuf::from(format!("archive-operation-source-{suffix}.sqlite"));
        let destination = PathBuf::from(format!("archive-operation-destination-{suffix}.tmp"));
        let lease = ArchiveFileOperationLease::register([source.clone(), destination.clone()])
            .expect("register archive file operation lease");

        assert!(is_archive_file_operation_active(&source));
        assert!(is_archive_file_operation_active(&destination));
        assert!(is_archive_file_operation_active(&PathBuf::from(format!(
            "{}-wal",
            source.display()
        ))));
        assert!(is_archive_file_operation_active(
            &source.with_extension("owner")
        ));
        assert!(ArchiveFileOperationLease::register([source.clone()]).is_none());
        assert!(ArchiveFileOperationLease::register([destination.clone()]).is_none());

        drop(lease);
        assert!(!is_archive_file_operation_active(&source));
        assert!(!is_archive_file_operation_active(&destination));
    }

    #[test]
    fn archive_file_operation_lease_allows_shared_readers_but_fences_writers() {
        let suffix = RETENTION_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let source = PathBuf::from(format!("archive-shared-source-{suffix}.sqlite.gz"));
        let first = ArchiveFileOperationLease::register_with_shared_reads(
            [PathBuf::from(format!(
                "archive-shared-first-{suffix}.sqlite"
            ))],
            [source.clone()],
        )
        .expect("register first shared archive reader");
        let second = ArchiveFileOperationLease::register_with_shared_reads(
            [PathBuf::from(format!(
                "archive-shared-second-{suffix}.sqlite"
            ))],
            [source.clone()],
        )
        .expect("register second shared archive reader");

        assert!(is_archive_file_operation_active(&source));
        assert!(ArchiveFileOperationLease::register([source.clone()]).is_none());

        drop(first);
        assert!(is_archive_file_operation_active(&source));
        drop(second);
        assert!(!is_archive_file_operation_active(&source));
    }

    #[tokio::test]
    async fn archive_file_work_timeout_keeps_worker_lease_until_drain() {
        let suffix = RETENTION_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let work_path = PathBuf::from(format!("archive-timeout-worker-{suffix}.sqlite"));
        let worker_started = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::new(AtomicBool::new(false));
        let worker_release = Arc::new(AtomicBool::new(false));
        let started = Arc::clone(&worker_started);
        let cancelled = Arc::clone(&worker_cancelled);
        let release = Arc::clone(&worker_release);
        let task = tokio::spawn(run_archive_file_work_with_deadline(
            ArchiveFileWorkRequest {
                operation: "test_archive_file_timeout",
                protected_paths: vec![work_path.clone()],
                shared_read_paths: Vec::new(),
                failure_cleanup_paths: Vec::new(),
                timeout_cleanup_paths: Vec::new(),
                deadline: None,
                timeout: Some(Duration::from_millis(20)),
                shutdown: None,
            },
            move |_, _, cancellation, _| {
                started.store(true, Ordering::Release);
                while !cancellation.load(Ordering::Acquire) && !release.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(1));
                }
                cancelled.store(cancellation.load(Ordering::Acquire), Ordering::Release);
                Ok(())
            },
        ));

        if tokio::time::timeout(Duration::from_secs(1), async {
            while !worker_started.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_err()
        {
            worker_release.store(true, Ordering::Release);
            panic!("archive worker did not start");
        }

        let result = match tokio::time::timeout(Duration::from_secs(1), task).await {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                worker_release.store(true, Ordering::Release);
                panic!("archive worker task failed: {error}");
            }
            Err(_) => {
                worker_release.store(true, Ordering::Release);
                panic!("archive worker timeout did not return");
            }
        };
        assert!(result.is_err());
        assert!(worker_cancelled.load(Ordering::Acquire));
        assert!(!is_archive_file_operation_active(&work_path));
    }

    #[tokio::test]
    async fn archive_file_work_cancellation_reaches_worker_before_lease_drain() {
        let suffix = RETENTION_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let work_path = PathBuf::from(format!("archive-cancel-worker-{suffix}.sqlite"));
        let worker_started = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::new(AtomicBool::new(false));
        let worker_force_release = Arc::new(AtomicBool::new(false));
        let started = Arc::clone(&worker_started);
        let cancelled = Arc::clone(&worker_cancelled);
        let force_release = Arc::clone(&worker_force_release);
        let task = tokio::spawn(run_archive_file_work_with_deadline(
            ArchiveFileWorkRequest {
                operation: "test_archive_file_cancellation",
                protected_paths: vec![work_path.clone()],
                shared_read_paths: Vec::new(),
                failure_cleanup_paths: Vec::new(),
                timeout_cleanup_paths: Vec::new(),
                deadline: None,
                timeout: None,
                shutdown: None,
            },
            move |_, _, cancellation, _| {
                started.store(true, Ordering::Release);
                while !cancellation.load(Ordering::Acquire)
                    && !force_release.load(Ordering::Acquire)
                {
                    std::thread::sleep(Duration::from_millis(1));
                }
                cancelled.store(cancellation.load(Ordering::Acquire), Ordering::Release);
                Ok(())
            },
        ));

        tokio::time::timeout(Duration::from_secs(1), async {
            while !worker_started.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("archive cancellation worker should start");
        task.abort();
        let _ = task.await;

        let drained = tokio::time::timeout(Duration::from_secs(1), async {
            while is_archive_file_operation_active(&work_path) {
                tokio::task::yield_now().await;
            }
        })
        .await;
        if drained.is_err() {
            worker_force_release.store(true, Ordering::Release);
            panic!("archive worker did not observe caller cancellation");
        }
        assert!(worker_cancelled.load(Ordering::Acquire));
    }
}
