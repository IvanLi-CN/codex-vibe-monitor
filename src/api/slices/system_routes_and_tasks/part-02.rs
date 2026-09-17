struct SystemStatusFilesystemScan {
    cancellation: CancellationToken,
    deadline: Instant,
    #[cfg(test)]
    checkpoint: Option<Arc<dyn Fn() + Send + Sync>>,
    #[cfg(test)]
    blocking_operation: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl SystemStatusFilesystemScan {
    fn new(cancellation: CancellationToken, deadline: Instant) -> Self {
        Self {
            cancellation,
            deadline,
            #[cfg(test)]
            checkpoint: None,
            #[cfg(test)]
            blocking_operation: None,
        }
    }

    #[cfg(test)]
    fn with_test_checkpoint(
        cancellation: CancellationToken,
        deadline: Instant,
        checkpoint: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self {
            cancellation,
            deadline,
            checkpoint: Some(checkpoint),
            blocking_operation: None,
        }
    }

    #[cfg(test)]
    fn with_test_blocking_operation(
        mut self,
        blocking_operation: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        self.blocking_operation = Some(blocking_operation);
        self
    }

    fn check(&self) -> Result<()> {
        #[cfg(test)]
        if let Some(checkpoint) = &self.checkpoint {
            checkpoint();
        }
        if self.cancellation.is_cancelled() {
            bail!("system status filesystem scan cancelled");
        }
        if Instant::now() >= self.deadline {
            bail!("system status filesystem scan exceeded its deadline");
        }
        Ok(())
    }

    fn before_filesystem_operation(&self) {
        #[cfg(test)]
        if let Some(blocking_operation) = &self.blocking_operation {
            blocking_operation();
        }
    }
}

#[derive(Debug)]
struct SystemStatusFilesystemBytes {
    archive_bytes: u64,
    database_bytes: u64,
    other_files_bytes: u64,
}

struct SystemStatusFilesystemScanInputs {
    archive_paths: Vec<String>,
    config: AppConfig,
    archive_dir: PathBuf,
    raw_dir: PathBuf,
}

fn count_file_size_with_scan(path: &Path, scan: &SystemStatusFilesystemScan) -> Result<u64> {
    scan.check()?;
    scan.before_filesystem_operation();
    Ok(fs::metadata(path).map(|meta| meta.len()).unwrap_or(0))
}

pub(crate) fn add_existing_raw_payload_bytes(
    raw_path: &str,
    fallback_root: Option<&Path>,
    seen_paths: &mut HashSet<PathBuf>,
    metric: &mut SystemStatusMetric,
) {
    let Some(candidate) = resolved_raw_path_read_candidates(raw_path, fallback_root)
        .into_iter()
        .find(|candidate| candidate.exists())
    else {
        return;
    };
    if !seen_paths.insert(candidate.clone()) {
        return;
    }
    metric.count = metric.count.saturating_add(1);
    metric.bytes = metric.bytes.saturating_add(count_file_size(&candidate));
}

pub(crate) fn collect_existing_raw_payload_metrics(
    rows: &[SystemRawBodyPathRow],
    fallback_root: Option<&Path>,
) -> (SystemStatusMetric, SystemStatusMetric, SystemStatusMetric) {
    let mut total_seen_paths = HashSet::new();
    let mut request_seen_paths = HashSet::new();
    let mut response_seen_paths = HashSet::new();
    let mut total = SystemStatusMetric::default();
    let mut request = SystemStatusMetric::default();
    let mut response = SystemStatusMetric::default();

    for row in rows {
        if let Some(raw_path) = row.request_raw_path.as_deref() {
            add_existing_raw_payload_bytes(
                raw_path,
                fallback_root,
                &mut request_seen_paths,
                &mut request,
            );
            add_existing_raw_payload_bytes(
                raw_path,
                fallback_root,
                &mut total_seen_paths,
                &mut total,
            );
        }
        if let Some(raw_path) = row.response_raw_path.as_deref() {
            add_existing_raw_payload_bytes(
                raw_path,
                fallback_root,
                &mut response_seen_paths,
                &mut response,
            );
            add_existing_raw_payload_bytes(
                raw_path,
                fallback_root,
                &mut total_seen_paths,
                &mut total,
            );
        }
    }

    (total, request, response)
}

fn count_database_bytes_with_scan(
    db_path: &Path,
    scan: &SystemStatusFilesystemScan,
) -> Result<u64> {
    let wal_path = PathBuf::from(format!("{}-wal", db_path.display()));
    let shm_path = PathBuf::from(format!("{}-shm", db_path.display()));
    Ok(count_file_size_with_scan(db_path, scan)?
        .saturating_add(count_file_size_with_scan(&wal_path, scan)?)
        .saturating_add(count_file_size_with_scan(&shm_path, scan)?))
}

fn sum_directory_bytes_with_scan(root: &Path, scan: &SystemStatusFilesystemScan) -> Result<u64> {
    let mut total = 0_u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        scan.check()?;
        scan.before_filesystem_operation();
        let Ok(entries) = fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            scan.check()?;
            let child = entry.path();
            scan.check()?;
            scan.before_filesystem_operation();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push(child),
                Ok(kind) if kind.is_file() => {
                    scan.check()?;
                    scan.before_filesystem_operation();
                    total =
                        total.saturating_add(entry.metadata().map(|meta| meta.len()).unwrap_or(0));
                }
                _ => {}
            }
        }
    }
    Ok(total)
}

fn sum_path_bytes_with_scan(path: &Path, scan: &SystemStatusFilesystemScan) -> Result<u64> {
    scan.check()?;
    scan.before_filesystem_operation();
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(metadata.len()),
        Ok(metadata) if metadata.is_dir() => sum_directory_bytes_with_scan(path, scan),
        _ => Ok(0),
    }
}

#[derive(Debug)]
struct SystemStatusFilesystemScanPermit {
    in_flight: Arc<AtomicBool>,
    abandoned: Arc<AtomicBool>,
}

impl SystemStatusFilesystemScanPermit {
    fn try_acquire(cache: &SystemStatusCacheState) -> Result<(Self, Arc<AtomicBool>)> {
        let in_flight = cache.filesystem_scan_in_flight.clone();
        if in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            debug!(
                filesystem_scan_in_flight = true,
                "system status filesystem scan deferred while an earlier scan is still running"
            );
            bail!("system status filesystem scan is already in flight");
        }
        let abandoned = Arc::new(AtomicBool::new(false));
        Ok((
            Self {
                in_flight,
                abandoned: abandoned.clone(),
            },
            abandoned,
        ))
    }
}

impl Drop for SystemStatusFilesystemScanPermit {
    fn drop(&mut self) {
        self.in_flight.store(false, Ordering::Release);
        if self.abandoned.load(Ordering::Acquire) {
            debug!("abandoned system status filesystem scan exited; scan admission released");
        }
    }
}

fn compute_other_files_bytes_with_scan(
    config: &AppConfig,
    archive_dir: &Path,
    raw_dir: &Path,
    scan: &SystemStatusFilesystemScan,
) -> Result<u64> {
    let db_path = &config.database_path;
    let db_wal_path = PathBuf::from(format!("{}-wal", db_path.display()));
    let db_shm_path = PathBuf::from(format!("{}-shm", db_path.display()));
    let mut seen = HashSet::new();

    // Keep "other files" scoped to runtime-owned storage that does not already
    // have a dedicated metric on the system status page.
    let mut total = 0_u64;
    for path in [config.xray_runtime_dir.clone()] {
        scan.check()?;
        if path.as_os_str().is_empty() || !seen.insert(path.clone()) {
            continue;
        }
        let candidate = path.as_path();
        if candidate != db_path
            && candidate != db_wal_path.as_path()
            && candidate != db_shm_path.as_path()
            && candidate != archive_dir
            && candidate != raw_dir
        {
            total = total.saturating_add(sum_path_bytes_with_scan(&path, scan)?);
        }
    }
    Ok(total)
}

fn collect_system_status_filesystem_bytes(
    inputs: SystemStatusFilesystemScanInputs,
    scan: SystemStatusFilesystemScan,
) -> Result<SystemStatusFilesystemBytes> {
    let SystemStatusFilesystemScanInputs {
        archive_paths,
        config,
        archive_dir,
        raw_dir,
    } = inputs;
    let mut seen_paths = HashSet::new();
    let mut archive_bytes = 0_u64;
    for path in archive_paths {
        scan.check()?;
        if seen_paths.insert(path.clone()) {
            archive_bytes =
                archive_bytes.saturating_add(count_file_size_with_scan(Path::new(&path), &scan)?);
        }
    }

    Ok(SystemStatusFilesystemBytes {
        archive_bytes,
        database_bytes: count_database_bytes_with_scan(&config.database_path, &scan)?,
        other_files_bytes: compute_other_files_bytes_with_scan(
            &config,
            &archive_dir,
            &raw_dir,
            &scan,
        )?,
    })
}

async fn collect_system_status_filesystem_bytes_in_blocking_task(
    archive_paths: Vec<String>,
    config: AppConfig,
    archive_dir: PathBuf,
    raw_dir: PathBuf,
    cache: &Arc<Mutex<SystemStatusCacheState>>,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<SystemStatusFilesystemBytes> {
    let scan = SystemStatusFilesystemScan::new(cancellation.clone(), deadline);
    collect_system_status_filesystem_bytes_in_blocking_task_with_scan(
        SystemStatusFilesystemScanInputs {
            archive_paths,
            config,
            archive_dir,
            raw_dir,
        },
        cache,
        cancellation,
        deadline,
        scan,
    )
    .await
}

async fn collect_system_status_filesystem_bytes_in_blocking_task_with_scan(
    inputs: SystemStatusFilesystemScanInputs,
    cache: &Arc<Mutex<SystemStatusCacheState>>,
    cancellation: &CancellationToken,
    deadline: Instant,
    scan: SystemStatusFilesystemScan,
) -> Result<SystemStatusFilesystemBytes> {
    ensure_system_status_filesystem_scan_active(cancellation, deadline)?;
    let (permit, abandoned) = {
        let cache =
            await_system_status_refresh_operation(cancellation, async { Ok(cache.lock().await) })
                .await?;
        ensure_system_status_filesystem_scan_active(cancellation, deadline)?;
        SystemStatusFilesystemScanPermit::try_acquire(&cache)?
    };
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        collect_system_status_filesystem_bytes(inputs, scan)
    });

    await_system_status_filesystem_scan_task(task, cancellation, deadline, &abandoned).await
}

fn ensure_system_status_filesystem_scan_active(
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<()> {
    if cancellation.is_cancelled() {
        bail!("system status filesystem scan cancelled");
    }
    if Instant::now() >= deadline {
        cancellation.cancel();
        bail!("system status filesystem scan exceeded its deadline");
    }
    Ok(())
}

async fn await_system_status_filesystem_scan_task(
    mut task: tokio::task::JoinHandle<Result<SystemStatusFilesystemBytes>>,
    cancellation: &CancellationToken,
    deadline: Instant,
    abandoned: &Arc<AtomicBool>,
) -> Result<SystemStatusFilesystemBytes> {
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => {
            abandoned.store(true, Ordering::Release);
            // Abort only prevents queued work. A started blocking filesystem call remains alive,
            // holding its permit until it returns so retries cannot accumulate blocked workers.
            task.abort();
            drop(task);
            warn!(
                abandon_reason = "cancelled",
                "system status filesystem scan abandoned; retaining its scan admission until it exits"
            );
            bail!("system status filesystem scan cancelled");
        }
        _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
            cancellation.cancel();
            abandoned.store(true, Ordering::Release);
            // See the cancellation branch: this does not claim to interrupt started OS I/O.
            task.abort();
            drop(task);
            warn!(
                abandon_reason = "deadline",
                "system status filesystem scan abandoned; retaining its scan admission until it exits"
            );
            bail!("system status filesystem scan exceeded its deadline");
        }
        result = &mut task => {
            let filesystem_bytes = result
                .map_err(|error| anyhow!("system status filesystem scan task failed: {error}"))??;
            // A blocking syscall may return after the timer wakes. Do not let its successful
            // result escape toward snapshot publication once this refresh is no longer valid.
            ensure_system_status_filesystem_scan_active(cancellation, deadline)?;
            Ok(filesystem_bytes)
        }
    }
}

async fn record_system_raw_payload_inventory_path(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    raw_path: &str,
    byte_size: i64,
    request_seen: bool,
    response_seen: bool,
) -> Result<(i64, i64, i64, i64, i64, i64)> {
    let existing = sqlx::query_as::<_, SystemRawPayloadInventoryPathRow>(
        r#"
        SELECT byte_size, request_seen, response_seen
        FROM system_raw_payload_inventory_paths
        WHERE raw_path = ?1
        "#,
    )
    .bind(raw_path)
    .fetch_optional(tx.as_mut())
    .await?;

    let Some(existing) = existing else {
        sqlx::query(
            r#"
            INSERT INTO system_raw_payload_inventory_paths (
                raw_path, byte_size, request_seen, response_seen
            )
            VALUES (?1, ?2, ?3, ?4)
            "#,
        )
        .bind(raw_path)
        .bind(byte_size)
        .bind(i64::from(request_seen))
        .bind(i64::from(response_seen))
        .execute(tx.as_mut())
        .await?;
        return Ok((
            1,
            byte_size,
            i64::from(request_seen),
            if request_seen { byte_size } else { 0 },
            i64::from(response_seen),
            if response_seen { byte_size } else { 0 },
        ));
    };

    let request_added = request_seen && existing.request_seen == 0;
    let response_added = response_seen && existing.response_seen == 0;
    if request_added || response_added {
        sqlx::query(
            r#"
            UPDATE system_raw_payload_inventory_paths
            SET request_seen = MAX(request_seen, ?2), response_seen = MAX(response_seen, ?3)
            WHERE raw_path = ?1
            "#,
        )
        .bind(raw_path)
        .bind(i64::from(request_seen))
        .bind(i64::from(response_seen))
        .execute(tx.as_mut())
        .await?;
    }
    Ok((
        0,
        0,
        i64::from(request_added),
        if request_added { existing.byte_size } else { 0 },
        i64::from(response_added),
        if response_added {
            existing.byte_size
        } else {
            0
        },
    ))
}
