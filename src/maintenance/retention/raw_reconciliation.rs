use super::*;

#[derive(Debug, FromRow)]
pub(crate) struct RetentionRawReconciliationRow {
    pub(crate) raw_path: String,
    pub(crate) file_identity: String,
    pub(crate) byte_size: i64,
    pub(crate) quarantined_at: String,
    pub(crate) release_pending: i64,
    pub(crate) release_pending_at: Option<String>,
    pub(crate) removal_evidence_persisted: i64,
}

#[derive(Debug, Default)]
pub(crate) struct RetentionRawDirectoryTraversal {
    pub(crate) root: Option<PathBuf>,
    pub(crate) directory: Option<ReadDir>,
    pub(crate) pending_candidates: VecDeque<RetentionRawDirectoryEntry>,
}

#[derive(Clone, Debug)]
pub(crate) struct RetentionRawDirectoryEntry {
    pub(crate) path: PathBuf,
}

pub(crate) struct RetentionRawDirectorySlice {
    pub(crate) directory: Option<ReadDir>,
    pub(crate) candidates: Vec<RetentionRawDirectoryEntry>,
    pub(crate) inspected_entries: usize,
    pub(crate) reached_end: bool,
    pub(crate) failures: usize,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RawOrphanSweepCandidateBudgetExpired;

impl std::fmt::Display for RawOrphanSweepCandidateBudgetExpired {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("raw orphan sweep candidate budget expired")
    }
}

impl std::error::Error for RawOrphanSweepCandidateBudgetExpired {}

pub(crate) fn raw_orphan_sweep_candidate_budget_expired() -> anyhow::Error {
    anyhow::Error::new(RawOrphanSweepCandidateBudgetExpired)
}

pub(crate) fn is_raw_orphan_sweep_candidate_budget_expired(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.is::<RawOrphanSweepCandidateBudgetExpired>())
}

pub(crate) struct RawOrphanSweepPassContext {
    pub(crate) _pressure_permit: crate::db_pressure::DbBackgroundPermit,
    pub(crate) candidate_deadline: Instant,
}

impl RawOrphanSweepPassContext {
    pub(crate) fn candidate_budget_expired(&self) -> bool {
        Instant::now() >= self.candidate_deadline
    }

    pub(crate) fn candidate_budget_remaining(&self) -> Option<Duration> {
        self.candidate_deadline
            .checked_duration_since(Instant::now())
    }

    pub(crate) async fn run_with_candidate_budget<T, F>(&self, future: F) -> Result<T>
    where
        F: Future<Output = Result<T>>,
    {
        let Some(remaining) = self.candidate_budget_remaining() else {
            return Err(raw_orphan_sweep_candidate_budget_expired());
        };
        tokio::time::timeout(remaining, future)
            .await
            .map_err(|_| raw_orphan_sweep_candidate_budget_expired())?
    }

    pub(crate) async fn try_write(
        &self,
        operation: &'static str,
    ) -> Result<crate::proxy_sqlite_write_coordinator::ProxySqliteWritePermit> {
        let Some(remaining) = self.candidate_budget_remaining() else {
            return Err(raw_orphan_sweep_candidate_budget_expired());
        };
        let admission_wait = remaining.min(RETENTION_FAIRNESS_INTERVAL);
        #[cfg(test)]
        if RETENTION_TEST_RAW_MIDPASS_WRITE_DENY
            .try_with(|deny| deny.load(std::sync::atomic::Ordering::Relaxed))
            .unwrap_or(false)
        {
            retention_record_defer(operation, "coordinator_wait");
            return Err(raw_orphan_sweep_admission_deferred(
                "maintenance_write",
                "coordinator_wait",
            ));
        }
        let coordinator = retention_write_coordinator_handle();
        let write_permit = match RETENTION_SHUTDOWN.try_with(Clone::clone) {
            Ok(shutdown) => tokio::time::timeout(
                admission_wait,
                coordinator.acquire_maintenance_cancellable(RETENTION_FAIRNESS_INTERVAL, &shutdown),
            )
            .await
            .ok()
            .flatten(),
            Err(_) => tokio::time::timeout(
                admission_wait,
                coordinator.acquire_maintenance(RETENTION_FAIRNESS_INTERVAL),
            )
            .await
            .ok(),
        };
        if let Some(write_permit) = write_permit {
            return Ok(write_permit);
        }
        if self.candidate_budget_expired() {
            return Err(raw_orphan_sweep_candidate_budget_expired());
        }
        let cause = if RETENTION_SHUTDOWN
            .try_with(|shutdown| shutdown.is_cancelled())
            .unwrap_or(false)
        {
            "shutdown"
        } else {
            "coordinator_wait"
        };
        retention_record_defer(operation, cause);
        Err(raw_orphan_sweep_admission_deferred(
            "maintenance_write",
            cause,
        ))
    }
}

pub(crate) async fn read_retention_raw_directory_slice_with_budget(
    root: &Path,
    directory: Option<ReadDir>,
) -> Result<RetentionRawDirectorySlice> {
    let root = root.to_owned();
    #[cfg(test)]
    retention_test_raw_directory_entry_event();
    // A ReadDir cannot be cancelled safely once moved into a blocking worker. Await the
    // single worker to preserve its position and avoid accumulating detached scans after a
    // slow filesystem call; the caller checks the budget again before admitting its results.
    tokio::task::spawn_blocking(move || read_retention_raw_directory_slice(&root, directory))
        .await
        .map_err(|error| anyhow!("raw directory scan worker failed: {error}"))?
}

pub(crate) async fn retention_raw_file_metadata_with_budget(
    context: &RawOrphanSweepPassContext,
    path: &Path,
) -> Result<io::Result<fs::Metadata>> {
    let path = path.to_owned();
    context
        .run_with_candidate_budget(async move {
            tokio::task::spawn_blocking(move || retention_raw_file_metadata(&path))
                .await
                .map_err(|error| anyhow!("raw metadata worker failed: {error}"))
        })
        .await
}

pub(crate) async fn acquire_raw_orphan_sweep_pass_context() -> Result<RawOrphanSweepPassContext> {
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

    if let Some(crate::db_pressure::DbPressureDenyReason::PressureCooldown { .. }) =
        pressure_gate.background_deny_reason()
    {
        retention_record_defer("raw_orphan_sweep", "pressure_cooldown");
        return Err(raw_orphan_sweep_admission_deferred(
            "background_slot",
            "pressure_cooldown",
        ));
    }

    let pressure_permit = pressure_gate
        .begin_priority_background_with_queue_wait("raw_orphan_sweep", RETENTION_FAIRNESS_INTERVAL)
        .await
        .map_err(|reason| {
            let cause = match reason {
                crate::db_pressure::DbPressureDenyReason::PressureCooldown { .. } => {
                    "pressure_cooldown"
                }
                crate::db_pressure::DbPressureDenyReason::BackgroundBusy => "background_busy",
            };
            retention_record_defer("raw_orphan_sweep", cause);
            raw_orphan_sweep_admission_deferred("background_slot", cause)
        })?;

    let coordinator = retention_write_coordinator_handle();
    let write_permit = match RETENTION_SHUTDOWN.try_with(Clone::clone) {
        Ok(shutdown) => tokio::time::timeout(
            RETENTION_FAIRNESS_INTERVAL,
            coordinator.acquire_maintenance_cancellable(RETENTION_FAIRNESS_INTERVAL, &shutdown),
        )
        .await
        .ok()
        .flatten(),
        Err(_) => tokio::time::timeout(
            RETENTION_FAIRNESS_INTERVAL,
            coordinator.acquire_maintenance(RETENTION_FAIRNESS_INTERVAL),
        )
        .await
        .ok(),
    };
    let Some(write_permit) = write_permit else {
        let cause = if RETENTION_SHUTDOWN
            .try_with(|shutdown| shutdown.is_cancelled())
            .unwrap_or(false)
        {
            "shutdown"
        } else {
            "coordinator_wait"
        };
        drop(pressure_permit);
        retention_record_defer("raw_orphan_sweep", cause);
        return Err(raw_orphan_sweep_admission_deferred(
            "maintenance_write",
            cause,
        ));
    };
    drop(write_permit);

    crate::task_runtime_observation::mark_managed_maintenance_work_started();
    Ok(RawOrphanSweepPassContext {
        _pressure_permit: pressure_permit,
        candidate_deadline: Instant::now() + RETENTION_RAW_RECONCILIATION_CANDIDATE_BUDGET,
    })
}

pub(crate) fn read_retention_raw_directory_slice(
    root: &Path,
    mut directory: Option<ReadDir>,
) -> Result<RetentionRawDirectorySlice> {
    if directory.is_none() {
        #[cfg(test)]
        retention_test_raw_directory_open_event();
        directory = match fs::read_dir(root) {
            Ok(directory) => Some(directory),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(RetentionRawDirectorySlice {
                    directory: None,
                    candidates: Vec::new(),
                    inspected_entries: 0,
                    reached_end: true,
                    failures: 0,
                });
            }
            Err(error) => {
                return Err(anyhow!(
                    "failed to open raw reconciliation directory ({})",
                    error.kind()
                ));
            }
        };
    }

    let mut candidates = Vec::with_capacity(RETENTION_RAW_RECONCILIATION_SCAN_BATCH);
    let mut inspected_entries = 0;
    let mut failures = 0;
    let mut reached_end = false;
    let directory_state = directory
        .as_mut()
        .expect("raw directory is opened or returned as absent");
    for _ in 0..RETENTION_RAW_RECONCILIATION_DIRECTORY_ENTRY_BATCH {
        #[cfg(test)]
        retention_test_raw_directory_entry_event();
        let Some(entry) = directory_state.next() else {
            reached_end = true;
            break;
        };
        inspected_entries += 1;
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                failures += 1;
                continue;
            }
        };
        let name = entry.file_name();
        if !retention_raw_reconciliation_supported_name(&name.to_string_lossy()) {
            continue;
        }
        match entry.file_type() {
            Ok(file_type) if file_type.is_file() => {
                candidates.push(RetentionRawDirectoryEntry { path: entry.path() });
                if candidates.len() >= RETENTION_RAW_RECONCILIATION_SCAN_BATCH {
                    break;
                }
            }
            Ok(_) => {}
            Err(_) => failures += 1,
        }
    }

    Ok(RetentionRawDirectorySlice {
        directory: if reached_end { None } else { directory },
        candidates,
        inspected_entries,
        reached_end,
        failures,
    })
}

pub(crate) fn retention_raw_reconciliation_supported_name(name: &str) -> bool {
    name.ends_with(".bin") || name.ends_with(".bin.gz") || name.ends_with(".bin.zst")
}

pub(crate) fn retention_raw_file_identity(metadata: &fs::Metadata) -> String {
    #[cfg(unix)]
    {
        format!(
            "{}:{}:{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.ctime(),
            metadata.ctime_nsec()
        )
    }
    #[cfg(not(unix))]
    {
        let modified = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_nanos())
            .unwrap_or_default();
        format!("{}:{modified}", metadata.len())
    }
}

pub(crate) fn retention_raw_quarantine_due(quarantined_at: &str) -> bool {
    parse_to_utc_datetime(quarantined_at).is_some_and(|observed_at| {
        Utc::now().signed_duration_since(observed_at)
            >= ChronoDuration::seconds(RETENTION_RECOVERY_QUARANTINE_GRACE_SECS)
    })
}

pub(crate) fn retention_raw_reference_path_variants(
    config: &AppConfig,
    candidate: &Path,
) -> Vec<String> {
    let candidate = normalize_path_for_compare(candidate);
    let configured_root = normalize_path_for_compare(&config.resolved_proxy_raw_dir());
    let mut paths = vec![candidate.to_string_lossy().into_owned()];
    if let Ok(relative) = candidate.strip_prefix(&configured_root)
        && let Some(database_parent) = config.database_path.parent()
    {
        let database_parent = normalize_path_for_compare(database_parent);
        if let Ok(raw_root_relative) = configured_root.strip_prefix(database_parent) {
            paths.push(
                raw_root_relative
                    .join(relative)
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    let initial = paths.clone();
    for path in initial {
        if let Some(alternate) = raw_payload_alternate_db_path(&path)
            && !paths.contains(&alternate)
        {
            paths.push(alternate);
        }
    }
    paths
}

pub(crate) async fn retention_raw_link_paths(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    candidates: &[PathBuf],
) -> Result<std::collections::HashSet<String>> {
    if candidates.is_empty() {
        return Ok(std::collections::HashSet::new());
    }
    if !crate::schema::proxy_raw_blob_link_seed_completed(pool).await? {
        bail!("raw blob-link seed completion marker is missing");
    }

    let mut paths = std::collections::HashSet::new();
    for candidate in candidates {
        paths.extend(retention_raw_reference_path_variants(config, candidate));
    }
    let paths = paths.into_iter().collect::<Vec<_>>();
    let placeholders = (1..=paths.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let query = format!(
        "SELECT DISTINCT raw_path FROM proxy_raw_payload_blob_links WHERE raw_path IN ({placeholders})"
    );
    let mut query = sqlx::query_scalar::<_, String>(&query);
    for path in &paths {
        query = query.bind(path);
    }
    Ok(query.fetch_all(pool).await?.into_iter().collect())
}

pub(crate) fn retention_raw_candidates_with_links(
    config: &AppConfig,
    candidates: &[PathBuf],
    link_paths: &std::collections::HashSet<String>,
    raw_path_fallback_root: Option<&Path>,
) -> std::collections::HashSet<PathBuf> {
    let mut referenced = std::collections::HashSet::new();
    for candidate in candidates {
        let candidate = normalize_path_for_compare(candidate);
        let variants = retention_raw_reference_path_variants(config, &candidate);
        for referenced_path in link_paths {
            if !variants.iter().any(|variant| variant == referenced_path) {
                continue;
            }
            let resolved =
                resolved_raw_path_read_candidates(referenced_path, raw_path_fallback_root);
            let Some(candidate_index) = resolved
                .iter()
                .position(|path| normalize_path_for_compare(path) == candidate)
            else {
                continue;
            };
            if candidate_index == 0
                || !resolved
                    .iter()
                    .take(candidate_index)
                    .any(|path| path.exists())
            {
                referenced.insert(candidate.clone());
                break;
            }
        }
    }
    referenced
}

pub(crate) async fn retention_raw_candidates_with_links_with_budget(
    context: &RawOrphanSweepPassContext,
    config: &AppConfig,
    candidates: &[PathBuf],
    link_paths: &std::collections::HashSet<String>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<std::collections::HashSet<PathBuf>> {
    let config = config.clone();
    let candidates = candidates.to_vec();
    let link_paths = link_paths.clone();
    let raw_path_fallback_root = raw_path_fallback_root.map(Path::to_owned);
    context
        .run_with_candidate_budget(async move {
            tokio::task::spawn_blocking(move || {
                Ok(retention_raw_candidates_with_links(
                    &config,
                    &candidates,
                    &link_paths,
                    raw_path_fallback_root.as_deref(),
                ))
            })
            .await
            .map_err(|error| anyhow!("raw reference fallback worker failed: {error}"))?
        })
        .await
}

pub(crate) async fn retention_raw_candidate_is_referenced(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    candidate: &Path,
    raw_path_fallback_root: Option<&Path>,
    context: &RawOrphanSweepPassContext,
) -> Result<bool> {
    let candidate = normalize_path_for_compare(candidate);
    let candidates = [candidate.clone()];
    let linked_paths = context
        .run_with_candidate_budget(retention_raw_link_paths(pool, config, &candidates))
        .await?;
    Ok(retention_raw_candidates_with_links_with_budget(
        context,
        config,
        &candidates,
        &linked_paths,
        raw_path_fallback_root,
    )
    .await?
    .contains(&candidate))
}

pub(crate) async fn load_retention_raw_reconciliation_row(
    pool: &Pool<Sqlite>,
    raw_path: &str,
) -> Result<Option<RetentionRawReconciliationRow>> {
    Ok(sqlx::query_as::<_, RetentionRawReconciliationRow>(
        r#"
        SELECT raw_path, file_identity, byte_size, quarantined_at, release_pending,
               release_pending_at, removal_evidence_persisted
        FROM retention_raw_reconciliation
        WHERE raw_path = ?1
        "#,
    )
    .bind(raw_path)
    .fetch_optional(pool)
    .await?)
}

pub(crate) async fn record_retention_raw_reconciliation_observation(
    pool: &Pool<Sqlite>,
    context: &RawOrphanSweepPassContext,
    raw_path: &str,
    file_identity: &str,
    byte_size: i64,
) -> Result<()> {
    let admission = context.try_write("raw_reconciliation_quarantine").await?;
    let result = context
        .run_with_candidate_budget(async {
            sqlx::query(
                r#"
                INSERT INTO retention_raw_reconciliation (
                    raw_path, file_identity, byte_size, quarantined_at, release_pending,
                    release_pending_at, removal_evidence_persisted
                )
                VALUES (?1, ?2, ?3, ?4, 0, NULL, 0)
                ON CONFLICT(raw_path) DO UPDATE SET
                    file_identity = excluded.file_identity,
                    byte_size = excluded.byte_size,
                    quarantined_at = CASE
                        WHEN retention_raw_reconciliation.file_identity = excluded.file_identity
                            AND retention_raw_reconciliation.byte_size = excluded.byte_size
                            THEN retention_raw_reconciliation.quarantined_at
                        ELSE excluded.quarantined_at
                    END,
                    release_pending = CASE
                        WHEN retention_raw_reconciliation.file_identity = excluded.file_identity
                            AND retention_raw_reconciliation.byte_size = excluded.byte_size
                            THEN retention_raw_reconciliation.release_pending
                        ELSE 0
                    END,
                    release_pending_at = CASE
                        WHEN retention_raw_reconciliation.file_identity = excluded.file_identity
                            AND retention_raw_reconciliation.byte_size = excluded.byte_size
                            THEN retention_raw_reconciliation.release_pending_at
                        ELSE NULL
                    END,
                    removal_evidence_persisted = CASE
                        WHEN retention_raw_reconciliation.file_identity = excluded.file_identity
                            AND retention_raw_reconciliation.byte_size = excluded.byte_size
                            THEN retention_raw_reconciliation.removal_evidence_persisted
                        ELSE 0
                    END,
                    updated_at = datetime('now')
                "#,
            )
            .bind(raw_path)
            .bind(file_identity)
            .bind(byte_size)
            .bind(format_utc_iso(Utc::now()))
            .execute(pool)
            .await?;
            Ok(())
        })
        .await;
    drop(admission);
    result
}

pub(crate) async fn mark_retention_raw_reconciliation_release_pending(
    pool: &Pool<Sqlite>,
    context: &RawOrphanSweepPassContext,
    raw_path: &str,
    file_identity: &str,
    byte_size: i64,
    release_pending_at: &str,
) -> Result<()> {
    context
        .run_with_candidate_budget(async {
            let result = sqlx::query(
                "UPDATE retention_raw_reconciliation
                 SET release_pending = 1,
                     release_pending_at = CASE
                         WHEN release_pending = 0 OR removal_evidence_persisted != 0
                             THEN ?4
                         ELSE COALESCE(release_pending_at, ?4)
                     END,
                     removal_evidence_persisted = 0,
                     updated_at = datetime('now')
                 WHERE raw_path = ?1 AND file_identity = ?2 AND byte_size = ?3",
            )
            .bind(raw_path)
            .bind(file_identity)
            .bind(byte_size)
            .bind(release_pending_at)
            .execute(pool)
            .await?;
            if result.rows_affected() != 1 {
                return Err(anyhow!(
                    "raw reconciliation release intent did not match the current ledger row"
                ));
            }
            Ok(())
        })
        .await
}

pub(crate) async fn clear_retention_raw_reconciliation_row(
    pool: &Pool<Sqlite>,
    context: &RawOrphanSweepPassContext,
    raw_path: &str,
    file_identity: Option<&str>,
) -> Result<()> {
    #[cfg(test)]
    if retention_test_raw_ledger_cleanup_should_fail() {
        return Err(anyhow!(
            "simulated raw reconciliation ledger cleanup failure"
        ));
    }
    let admission = context
        .try_write("raw_reconciliation_ledger_cleanup")
        .await?;
    let result = context
        .run_with_candidate_budget(async {
            if let Some(file_identity) = file_identity {
                sqlx::query(
                    "DELETE FROM retention_raw_reconciliation WHERE raw_path = ?1 AND file_identity = ?2",
                )
                .bind(raw_path)
                .bind(file_identity)
                .execute(pool)
                .await?;
            } else {
                sqlx::query("DELETE FROM retention_raw_reconciliation WHERE raw_path = ?1")
                    .bind(raw_path)
                    .execute(pool)
                    .await?;
            }
            Ok(())
        })
        .await;
    drop(admission);
    result
}

pub(crate) async fn rotate_retention_raw_reconciliation_pending_row(
    pool: &Pool<Sqlite>,
    context: &RawOrphanSweepPassContext,
    row: &RetentionRawReconciliationRow,
) -> Result<()> {
    let admission = context
        .try_write("raw_reconciliation_pending_rotation")
        .await?;
    let result = context
        .run_with_candidate_budget(async {
            sqlx::query(
                "UPDATE retention_raw_reconciliation
                 SET updated_at = strftime('%Y-%m-%d %H:%M:%f', 'now')
                 WHERE raw_path = ?1 AND file_identity = ?2 AND release_pending != 0",
            )
            .bind(&row.raw_path)
            .bind(&row.file_identity)
            .execute(pool)
            .await?;
            Ok(())
        })
        .await;
    drop(admission);
    result
}

pub(crate) async fn cleanup_missing_retention_raw_reconciliation_rows(
    pool: &Pool<Sqlite>,
    context: &RawOrphanSweepPassContext,
    raw_root: &Path,
    limit: usize,
) -> Result<(usize, usize, bool)> {
    let cursor = context
        .run_with_candidate_budget(async {
            Ok(sqlx::query_scalar::<_, String>(
                "SELECT cursor FROM retention_recovery_cursors WHERE scope = ?1",
            )
            .bind(RETENTION_RAW_RECONCILIATION_SCOPE)
            .fetch_optional(pool)
            .await?
            .unwrap_or_default())
        })
        .await?;
    let pending_limit = RETENTION_RAW_RECONCILIATION_PENDING_BATCH.min(limit);
    let regular_limit = limit.saturating_sub(pending_limit);
    let mut pending_rows = if pending_limit == 0 {
        Vec::new()
    } else {
        context
            .run_with_candidate_budget(async {
                Ok(sqlx::query_as::<_, RetentionRawReconciliationRow>(
                    r#"
                    SELECT raw_path, file_identity, byte_size, quarantined_at, release_pending,
                           release_pending_at, removal_evidence_persisted
                    FROM retention_raw_reconciliation
                    WHERE release_pending != 0
                    ORDER BY updated_at, raw_path
                    LIMIT ?1
                    "#,
                )
                .bind((pending_limit + 1) as i64)
                .fetch_all(pool)
                .await?)
            })
            .await?
    };
    let pending_has_more = pending_rows.len() > pending_limit;
    if pending_has_more {
        pending_rows.truncate(pending_limit);
    }
    let mut regular_rows = if regular_limit == 0 {
        Vec::new()
    } else {
        context
            .run_with_candidate_budget(async {
                Ok(sqlx::query_as::<_, RetentionRawReconciliationRow>(
                    r#"
                    SELECT raw_path, file_identity, byte_size, quarantined_at, release_pending,
                           release_pending_at, removal_evidence_persisted
                    FROM retention_raw_reconciliation
                    WHERE release_pending = 0 AND raw_path > ?1
                    ORDER BY raw_path
                    LIMIT ?2
                    "#,
                )
                .bind(&cursor)
                .bind((regular_limit + 1) as i64)
                .fetch_all(pool)
                .await?)
            })
            .await?
    };
    let regular_has_more = regular_rows.len() > regular_limit;
    if regular_has_more {
        regular_rows.truncate(regular_limit);
    }
    let regular_rows_empty = regular_rows.is_empty();
    let mut rows = pending_rows;
    rows.extend(regular_rows);
    let mut failures = 0;
    let mut processed_rows = 0;
    let mut progressed = None;
    let mut budget_exhausted = false;
    for row in &rows {
        if context.candidate_budget_expired() {
            budget_exhausted = true;
            break;
        }
        processed_rows += 1;
        let candidate = normalize_path_for_compare(Path::new(&row.raw_path));
        let metadata =
            retention_raw_file_metadata_with_budget(context, Path::new(&row.raw_path)).await?;
        let (metadata_missing, metadata_non_file, metadata_failed) = match metadata {
            Err(error) if error.kind() == io::ErrorKind::NotFound => (true, false, false),
            Ok(metadata) if !metadata.file_type().is_file() => (false, true, false),
            Err(_) => {
                failures += 1;
                (false, false, true)
            }
            Ok(_) => (false, false, false),
        };
        if metadata_failed {
            continue;
        }
        let metadata_missing_or_non_file = metadata_missing || metadata_non_file;
        if !candidate.starts_with(raw_root) {
            if row.release_pending != 0 && !metadata_missing_or_non_file {
                if let Err(error) =
                    rotate_retention_raw_reconciliation_pending_row(pool, context, row).await
                {
                    if is_raw_orphan_sweep_candidate_budget_expired(&error) {
                        budget_exhausted = true;
                        break;
                    }
                    if is_retention_write_deferred(&error) {
                        return Err(error);
                    }
                    failures += 1;
                }
                continue;
            }
            if row.release_pending != 0
                && metadata_missing
                && let Err(error) =
                    persist_missing_raw_reconciliation_release_evidence(pool, context, row).await
            {
                if is_raw_orphan_sweep_candidate_budget_expired(&error) {
                    budget_exhausted = true;
                    break;
                }
                if is_retention_write_deferred(&error) {
                    return Err(error);
                }
                failures += 1;
                continue;
            }
            if let Err(error) = clear_retention_raw_reconciliation_row(
                pool,
                context,
                &row.raw_path,
                Some(&row.file_identity),
            )
            .await
            {
                if is_raw_orphan_sweep_candidate_budget_expired(&error) {
                    budget_exhausted = true;
                    break;
                }
                if is_retention_write_deferred(&error) {
                    return Err(error);
                }
                failures += 1;
                continue;
            }
            if row.release_pending == 0 {
                progressed = Some(row.raw_path.as_str());
            }
            continue;
        }
        if metadata_missing_or_non_file {
            if row.release_pending != 0
                && metadata_missing
                && let Err(error) =
                    persist_missing_raw_reconciliation_release_evidence(pool, context, row).await
            {
                if is_raw_orphan_sweep_candidate_budget_expired(&error) {
                    budget_exhausted = true;
                    break;
                }
                if is_retention_write_deferred(&error) {
                    return Err(error);
                }
                failures += 1;
                continue;
            }
            if let Err(error) = clear_retention_raw_reconciliation_row(
                pool,
                context,
                &row.raw_path,
                Some(&row.file_identity),
            )
            .await
            {
                if is_raw_orphan_sweep_candidate_budget_expired(&error) {
                    budget_exhausted = true;
                    break;
                }
                if is_retention_write_deferred(&error) {
                    return Err(error);
                }
                failures += 1;
                continue;
            }
        } else if row.release_pending != 0
            && let Err(error) =
                rotate_retention_raw_reconciliation_pending_row(pool, context, row).await
        {
            if is_raw_orphan_sweep_candidate_budget_expired(&error) {
                budget_exhausted = true;
                break;
            }
            if is_retention_write_deferred(&error) {
                return Err(error);
            }
            failures += 1;
            continue;
        }
        if row.release_pending == 0 {
            progressed = Some(row.raw_path.as_str());
        }
    }

    if let Some(progress) = progressed {
        let admission = context.try_write("retention_recovery_cursor").await?;
        let result = context
            .run_with_candidate_budget(async {
                sqlx::query(
                    r#"
                    INSERT INTO retention_recovery_cursors (scope, cursor, updated_at)
                    VALUES (?1, ?2, datetime('now'))
                    ON CONFLICT(scope) DO UPDATE SET
                        cursor = CASE
                            WHEN retention_recovery_cursors.cursor = ?3
                                AND excluded.cursor > retention_recovery_cursors.cursor
                                THEN excluded.cursor
                            ELSE retention_recovery_cursors.cursor
                        END,
                        updated_at = CASE
                            WHEN retention_recovery_cursors.cursor = ?3
                                AND excluded.cursor > retention_recovery_cursors.cursor
                                THEN excluded.updated_at
                            ELSE retention_recovery_cursors.updated_at
                        END
                    "#,
                )
                .bind(RETENTION_RAW_RECONCILIATION_SCOPE)
                .bind(progress)
                .bind(&cursor)
                .execute(pool)
                .await?;
                Ok(())
            })
            .await;
        drop(admission);
        result?;
    } else if regular_rows_empty && !cursor.is_empty() {
        let admission = context.try_write("retention_recovery_cursor").await?;
        let result = context
            .run_with_candidate_budget(async {
                sqlx::query(
                    "UPDATE retention_recovery_cursors SET cursor = '', updated_at = datetime('now') \
                     WHERE scope = ?1 AND cursor = ?2",
                )
                .bind(RETENTION_RAW_RECONCILIATION_SCOPE)
                .bind(&cursor)
                .execute(pool)
                .await?;
                Ok(())
            })
            .await;
        drop(admission);
        result?;
    }

    let has_more = budget_exhausted || pending_has_more || regular_has_more;
    Ok((processed_rows, failures, has_more))
}

pub(crate) async fn persist_raw_orphan_sweep_removal_evidence_after_admission(
    pool: &Pool<Sqlite>,
    context: &RawOrphanSweepPassContext,
    raw_path: &str,
    file_identity: &str,
    removed: usize,
    removed_bytes: u64,
) -> Result<()> {
    #[cfg(test)]
    if retention_test_raw_removal_evidence_should_fail() {
        return Err(anyhow!(
            "simulated raw removal evidence persistence failure"
        ));
    }
    context
        .run_with_candidate_budget(async {
            let removal_at = format_utc_iso(Utc::now());
            let mut transaction = pool.begin().await?;
            let persisted: Option<i64> = sqlx::query_scalar(
                "SELECT removal_evidence_persisted
                 FROM retention_raw_reconciliation
                 WHERE raw_path = ?1 AND file_identity = ?2 AND release_pending != 0",
            )
            .bind(raw_path)
            .bind(file_identity)
            .fetch_optional(&mut *transaction)
            .await?;
            let Some(persisted) = persisted else {
                return Err(anyhow!(
                    "raw reconciliation release evidence did not match the current ledger row"
                ));
            };
            if persisted != 0 {
                transaction.commit().await?;
                return Ok(());
            }

            sqlx::query(
                "INSERT OR IGNORE INTO retention_recovery_cursors (scope, cursor) VALUES (?1, '')",
            )
            .bind(RETENTION_RAW_RECONCILIATION_SCOPE)
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                "UPDATE retention_recovery_cursors
                 SET last_nonzero_removal_at = CASE
                         WHEN last_nonzero_removal_at IS NULL
                              OR last_nonzero_removal_at <= ?4
                             THEN ?4
                         ELSE last_nonzero_removal_at
                     END,
                     last_nonzero_removal = CASE
                         WHEN last_nonzero_removal_at IS NULL
                              OR last_nonzero_removal_at <= ?4
                             THEN ?2
                         ELSE last_nonzero_removal
                     END,
                     last_nonzero_removal_bytes = CASE
                         WHEN last_nonzero_removal_at IS NULL
                              OR last_nonzero_removal_at <= ?4
                             THEN ?3
                         ELSE last_nonzero_removal_bytes
                     END,
                     updated_at = datetime('now')
                 WHERE scope = ?1",
            )
            .bind(RETENTION_RAW_RECONCILIATION_SCOPE)
            .bind(removed as i64)
            .bind(removed_bytes.min(i64::MAX as u64) as i64)
            .bind(&removal_at)
            .execute(&mut *transaction)
            .await?;
            let result = sqlx::query(
                "UPDATE retention_raw_reconciliation
                 SET removal_evidence_persisted = 1, updated_at = datetime('now')
                 WHERE raw_path = ?1 AND file_identity = ?2 AND release_pending != 0
                   AND removal_evidence_persisted = 0",
            )
            .bind(raw_path)
            .bind(file_identity)
            .execute(&mut *transaction)
            .await?;
            if result.rows_affected() != 1 {
                return Err(anyhow!(
                    "raw reconciliation release evidence marker did not match the current ledger row"
                ));
            }
            transaction.commit().await?;
            Ok(())
        })
        .await
}

pub(crate) async fn persist_pending_raw_reconciliation_release_evidence_after_admission(
    pool: &Pool<Sqlite>,
    context: &RawOrphanSweepPassContext,
    row: &RetentionRawReconciliationRow,
) -> Result<()> {
    if row.removal_evidence_persisted != 0 {
        return Ok(());
    }
    let removed_bytes = u64::try_from(row.byte_size)
        .map_err(|_| anyhow!("raw reconciliation byte size is negative"))?;
    if context.candidate_budget_expired() {
        return Err(raw_orphan_sweep_candidate_budget_expired());
    }
    match sync_retention_raw_parent(Path::new(&row.raw_path)) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(anyhow!(error).context("failed to sync raw reconciliation parent"));
        }
    }
    persist_raw_orphan_sweep_removal_evidence_after_admission(
        pool,
        context,
        &row.raw_path,
        &row.file_identity,
        1,
        removed_bytes,
    )
    .await
}

pub(crate) async fn persist_missing_raw_reconciliation_release_evidence(
    pool: &Pool<Sqlite>,
    context: &RawOrphanSweepPassContext,
    row: &RetentionRawReconciliationRow,
) -> Result<()> {
    let admission = context
        .try_write("raw_reconciliation_release_evidence")
        .await?;
    let result =
        persist_pending_raw_reconciliation_release_evidence_after_admission(pool, context, row)
            .await;
    drop(admission);
    result
}
