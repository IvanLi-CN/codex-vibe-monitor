use super::*;
use crate::observability::contention::RetentionPhase;
use crate::observability::contention::{RecoveryDiagnostic, record_recovery};
use sqlx::SqliteConnection;

struct RetentionDiagnosticRecorder {
    outcome: Option<RecoveryDiagnostic>,
    committed_rows: usize,
    emitted: bool,
}

impl RetentionDiagnosticRecorder {
    fn new() -> Self {
        Self {
            outcome: None,
            committed_rows: 0,
            emitted: false,
        }
    }

    fn committed(&mut self, rows: usize) {
        self.committed_rows = self.committed_rows.saturating_add(rows);
    }

    fn mark(&mut self, outcome: RecoveryDiagnostic) {
        self.outcome = Some(outcome);
    }

    fn mark_budget(&mut self) {
        retention_record_defer("retention_batch", "retention_work_budget");
        self.mark(RecoveryDiagnostic::RetentionBudgetExhausted);
    }

    fn mark_proof_unavailable(&mut self) {
        self.mark(RecoveryDiagnostic::RetentionProofUnavailable);
    }

    fn mark_stop(&mut self) {
        let (outcome, reason) =
            if retention_run_shutdown_token().is_some_and(|shutdown| shutdown.is_cancelled()) {
                (
                    RecoveryDiagnostic::RetentionCancelled,
                    "shutdown".to_string(),
                )
            } else if retention_run_budget_expired()
                || matches!(
                    retention_run_defer_reason().as_deref(),
                    Some("retention_work_budget") | Some("archive_design_budget")
                )
            {
                (
                    RecoveryDiagnostic::RetentionBudgetExhausted,
                    "retention_work_budget".to_string(),
                )
            } else {
                (
                    RecoveryDiagnostic::RetentionPressureDeferred,
                    retention_run_defer_reason()
                        .unwrap_or_else(|| "retention_write_admission".to_string()),
                )
            };
        retention_record_defer("retention_batch", reason);
        self.mark(outcome);
    }

    fn finish(&mut self) {
        let outcome = self.outcome.unwrap_or_else(|| {
            if self.committed_rows > 0 {
                RecoveryDiagnostic::RetentionCommitted
            } else if retention_run_budget_expired() {
                RecoveryDiagnostic::RetentionBudgetExhausted
            } else if retention_run_defer_reason().is_some() {
                RecoveryDiagnostic::RetentionPressureDeferred
            } else {
                RecoveryDiagnostic::RetentionNoWork
            }
        });
        self.emit(outcome);
    }

    fn emit(&mut self, outcome: RecoveryDiagnostic) {
        if !self.emitted {
            record_recovery(outcome);
            self.emitted = true;
        }
    }
}

impl Drop for RetentionDiagnosticRecorder {
    fn drop(&mut self) {
        if self.emitted {
            return;
        }
        let outcome = self.outcome.unwrap_or_else(|| {
            if retention_run_shutdown_token().is_some_and(|shutdown| shutdown.is_cancelled()) {
                RecoveryDiagnostic::RetentionCancelled
            } else {
                RecoveryDiagnostic::RetentionFailed
            }
        });
        self.emit(outcome);
    }
}

fn mark_error(diagnostic: &mut RetentionDiagnosticRecorder, error: &anyhow::Error) {
    let message = error.to_string();
    if is_retention_archive_file_operation_budget_exhausted(error) {
        diagnostic.mark_budget();
    } else if message.contains("archive artifact")
        || message.contains("source identity")
        || message.contains("summary archive")
        || message.contains("semantic proof")
    {
        diagnostic.mark_proof_unavailable();
    } else if retention_run_shutdown_token().is_some_and(|shutdown| shutdown.is_cancelled())
        || retention_run_budget_expired()
        || is_retention_write_deferred(error)
        || crate::is_sqlite_lock_error(error)
    {
        diagnostic.mark_stop();
    }
}

const INVOCATION_ARCHIVE_SELECTION_COLUMNS: &str = r#"
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
    NULL AS payload,
    request_raw_path,
    response_raw_path
"#;

async fn load_invocation_archive_candidate_metadata(
    pool: &Pool<Sqlite>,
    cutoff: &str,
    source_max_invocation_id: Option<i64>,
    candidate_limit: Option<usize>,
) -> Result<Vec<InvocationArchiveCandidate>> {
    // Selection only needs metadata and SQL-computed row sizes. Full payloads are loaded after
    // the 16 MiB/1000-row selector has chosen the bounded source IDs.
    let sql = format!(
        "SELECT {INVOCATION_ARCHIVE_SELECTION_COLUMNS}
         FROM codex_invocations
         WHERE occurred_at < ?1
           AND (?2 IS NULL OR id <= ?2)
         ORDER BY occurred_at ASC, id ASC{}",
        if candidate_limit.is_some() {
            "\n         LIMIT ?3"
        } else {
            ""
        }
    );
    let candidates = if let Some(candidate_limit) = candidate_limit {
        sqlx::query_as::<_, InvocationArchiveCandidate>(&sql)
            .bind(cutoff)
            .bind(source_max_invocation_id)
            .bind(candidate_limit as i64)
            .fetch_all(pool)
            .await?
    } else {
        sqlx::query_as::<_, InvocationArchiveCandidate>(&sql)
            .bind(cutoff)
            .bind(source_max_invocation_id)
            .fetch_all(pool)
            .await?
    };
    Ok(candidates)
}

async fn load_selected_invocation_archive_candidates(
    pool: &Pool<Sqlite>,
    ids: &[i64],
) -> Result<Vec<InvocationArchiveCandidate>> {
    let loaded = load_invocation_archive_candidates_by_ids(pool, ids).await?;
    if loaded.len() != ids.len() {
        bail!("retention task-local archive source selection changed before conversion");
    }
    let mut by_id = loaded
        .into_iter()
        .map(|candidate| (candidate.id, candidate))
        .collect::<HashMap<_, _>>();
    ids.iter()
        .map(|id| {
            by_id
                .remove(id)
                .context(
                    "retention task-local archive source selection changed before conversion: selected candidate unavailable",
                )
        })
        .collect()
}

async fn select_invocation_archive_group(
    pool: &Pool<Sqlite>,
    group: Vec<InvocationArchiveCandidate>,
) -> Result<(Vec<i64>, Vec<InvocationArchiveCandidate>)> {
    let candidate_ids = group.iter().map(|row| row.id).collect::<Vec<_>>();
    let row_sizes = archive_source_row_sizes(
        pool,
        archive_table_spec("codex_invocations"),
        &candidate_ids,
    )
    .await?;
    let selected_group = select_archive_batch(group, |candidate| {
        row_sizes.get(&candidate.id).copied().unwrap_or(512)
    });
    let ids = selected_group
        .iter()
        .map(|candidate| candidate.id)
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return Ok((ids, Vec::new()));
    }
    let group = load_selected_invocation_archive_candidates(pool, &ids).await?;
    Ok((ids, group))
}

fn invocation_archive_snapshot_pages(
    group: &[InvocationArchiveCandidate],
) -> Result<Vec<TaskArchiveSnapshotPage>> {
    let mut rows = group
        .iter()
        .map(invocation_archive_candidate_to_hourly_source_record)
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    let invoke_ids_by_row_id = group
        .iter()
        .map(|candidate| (candidate.id, candidate.invoke_id.clone()))
        .collect::<HashMap<_, _>>();

    rows.chunks(SUMMARY_ARCHIVE_SNAPSHOT_MAX_RECORDS)
        .map(|rows| {
            let first = rows.first().context("Summary archive page is empty")?;
            let last = rows.last().context("Summary archive page is empty")?;
            Ok(TaskArchiveSnapshotPage {
                coverage_start: first.occurred_at.clone(),
                coverage_end: last.occurred_at.clone(),
                row_count: u32::try_from(rows.len())
                    .context("Summary archive page row count overflow")?,
                payload: encode_summary_archive_snapshot_v2_payload(rows, &invoke_ids_by_row_id)?,
            })
        })
        .collect()
}

async fn persist_invocation_segment_summary_pages(
    pool: &Pool<Sqlite>,
    outcome: &ArchiveBatchOutcome,
    pages: &[TaskArchiveSnapshotPage],
) -> Result<bool> {
    let Some(batch_id) = stage_task_summary_archive_manifest(
        pool,
        outcome,
        Some(&outcome.sha256),
        Some(outcome.row_count),
    )
    .await?
    else {
        return Ok(false);
    };
    for (page_index, page) in pages.iter().cloned().enumerate() {
        if store_task_summary_snapshot_page(
            pool,
            outcome,
            batch_id,
            u32::try_from(page_index).context("Summary archive page index overflow")?,
            page,
        )
        .await?
        .is_none()
        {
            return Ok(false);
        }
        #[cfg(test)]
        if let Ok(counter) = crate::maintenance::RETENTION_TEST_TASK_ARCHIVE_SNAPSHOT_PAGE_STORES
            .try_with(Arc::clone)
        {
            counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
    Ok(true)
}

async fn repair_partial_invocation_archive(
    pool: &Pool<Sqlite>,
    outcome: &ArchiveBatchOutcome,
    selected_group: &[InvocationArchiveCandidate],
    committed_rows: usize,
) -> Result<()> {
    if committed_rows == 0 || committed_rows >= selected_group.len() {
        return Ok(());
    }

    let remove_ids = selected_group
        .iter()
        .skip(committed_rows)
        .map(|candidate| candidate.id)
        .collect::<Vec<_>>();
    let old_sha256 = outcome.sha256.clone();
    let batch_id = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM archive_batches
         WHERE dataset = ?1 AND month_key = ?2 AND file_path = ?3 AND sha256 = ?4
         ORDER BY id DESC LIMIT 1",
    )
    .bind(outcome.dataset)
    .bind(&outcome.month_key)
    .bind(&outcome.file_path)
    .bind(&old_sha256)
    .fetch_optional(pool)
    .await?
    .context("partial invocation archive manifest is missing")?;

    let (temporary_file_path, row_count, sha256) =
        crate::maintenance::trim_archive_batch_file_by_ids(outcome, &remove_ids).await?;
    let mut repaired_outcome = outcome.clone();
    repaired_outcome.row_count = row_count;
    repaired_outcome.sha256 = sha256;

    if let Err(error) = crate::maintenance::replace_archive_batch_file_with_cleanup_serialization(
        pool,
        &repaired_outcome,
        &temporary_file_path,
        &old_sha256,
    )
    .await
    {
        remove_temp_sqlite_artifacts(&temporary_file_path);
        return Err(error.context("publish partial invocation archive repair"));
    }

    // The file replacement changes the manifest identity. Remove the old page/proof set before
    // rebuilding it from the repaired artifact, so a stale proof can never authorize the new
    // source boundary.
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::query("DELETE FROM summary_archive_snapshot_v2_proof WHERE archive_batch_id = ?1")
        .bind(batch_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM summary_archive_snapshot WHERE archive_batch_id = ?1")
        .bind(batch_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM summary_coverage_obligation WHERE archive_batch_id = ?1")
        .bind(batch_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "UPDATE archive_batches
         SET coverage_start_at = NULL, coverage_end_at = NULL
         WHERE id = ?1 AND sha256 = ?2",
    )
    .bind(batch_id)
    .bind(&repaired_outcome.sha256)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;

    let pages = crate::maintenance::rebuild_task_month_snapshot_pages_for_archive(
        pool,
        &repaired_outcome,
        batch_id,
    )
    .await?;
    let first_page = pages
        .first()
        .context("partial archive repair produced no Summary pages")?;
    let last_page = pages
        .last()
        .context("partial archive repair produced no Summary pages")?;
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::query(
        "UPDATE archive_batches
         SET coverage_start_at = ?1, coverage_end_at = ?2
         WHERE id = ?3 AND sha256 = ?4",
    )
    .bind(&first_page.coverage_start)
    .bind(&last_page.coverage_end)
    .bind(batch_id)
    .bind(&repaired_outcome.sha256)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;

    let proof =
        crate::prepare_verified_summary_archive_snapshot(pool, batch_id, &repaired_outcome.sha256)
            .await?;
    crate::publish_verified_summary_archive_snapshot(pool, &proof).await?;

    // Partial repair replaces the published file and therefore changes its manifest SHA. Refresh
    // every replay marker after that replacement so the rollup proof stays bound to the repaired
    // artifact rather than the pre-repair file identity.
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    for target in INVOCATION_HOURLY_ROLLUP_TARGETS {
        mark_hourly_rollup_archive_replayed_tx(
            &mut transaction,
            target,
            repaired_outcome.dataset,
            &repaired_outcome.file_path,
        )
        .await?;
    }
    transaction.commit().await?;
    Ok(())
}

async fn repair_partial_timestamped_archive(
    pool: &Pool<Sqlite>,
    outcome: &ArchiveBatchOutcome,
    selected_group: &[TimestampedArchiveCandidate],
    committed_rows: usize,
) -> Result<()> {
    if committed_rows == 0 || committed_rows >= selected_group.len() {
        return Ok(());
    }

    let remove_ids = selected_group
        .iter()
        .skip(committed_rows)
        .map(|candidate| candidate.id)
        .collect::<Vec<_>>();
    let old_sha256 = outcome.sha256.clone();
    let (temporary_file_path, row_count, sha256) =
        crate::maintenance::trim_archive_batch_file_by_ids(outcome, &remove_ids).await?;
    let mut repaired_outcome = outcome.clone();
    repaired_outcome.row_count = row_count;
    repaired_outcome.sha256 = sha256;

    if let Err(error) = crate::maintenance::replace_archive_batch_file_with_cleanup_serialization(
        pool,
        &repaired_outcome,
        &temporary_file_path,
        &old_sha256,
    )
    .await
    {
        remove_temp_sqlite_artifacts(&temporary_file_path);
        return Err(error.context("publish partial timestamped archive repair"));
    }
    Ok(())
}

enum PartialArchiveRepairPlan {
    Invocation {
        pool: Pool<Sqlite>,
        outcome: ArchiveBatchOutcome,
        selected_group: Vec<InvocationArchiveCandidate>,
    },
    Timestamped {
        pool: Pool<Sqlite>,
        outcome: ArchiveBatchOutcome,
        selected_group: Vec<TimestampedArchiveCandidate>,
    },
}

struct PartialArchiveRepairState {
    committed_rows: std::sync::atomic::AtomicUsize,
    repair_claimed: std::sync::atomic::AtomicBool,
    cleanup_finished: std::sync::Arc<tokio::sync::Notify>,
    cleanup_done: std::sync::Arc<std::sync::atomic::AtomicBool>,
    plan: PartialArchiveRepairPlan,
}

struct PartialArchiveRepairOwner {
    state: std::sync::Arc<PartialArchiveRepairState>,
}

impl PartialArchiveRepairOwner {
    fn invocation(
        pool: &Pool<Sqlite>,
        outcome: &ArchiveBatchOutcome,
        selected_group: &[InvocationArchiveCandidate],
        cleanup_signal: (
            std::sync::Arc<tokio::sync::Notify>,
            std::sync::Arc<std::sync::atomic::AtomicBool>,
        ),
    ) -> Self {
        Self::new(
            PartialArchiveRepairPlan::Invocation {
                pool: pool.clone(),
                outcome: outcome.clone(),
                selected_group: selected_group.to_vec(),
            },
            cleanup_signal,
        )
    }

    fn timestamped(
        pool: &Pool<Sqlite>,
        outcome: &ArchiveBatchOutcome,
        selected_group: &[TimestampedArchiveCandidate],
        cleanup_signal: (
            std::sync::Arc<tokio::sync::Notify>,
            std::sync::Arc<std::sync::atomic::AtomicBool>,
        ),
    ) -> Self {
        Self::new(
            PartialArchiveRepairPlan::Timestamped {
                pool: pool.clone(),
                outcome: outcome.clone(),
                selected_group: selected_group.to_vec(),
            },
            cleanup_signal,
        )
    }

    fn new(
        plan: PartialArchiveRepairPlan,
        (cleanup_finished, cleanup_done): (
            std::sync::Arc<tokio::sync::Notify>,
            std::sync::Arc<std::sync::atomic::AtomicBool>,
        ),
    ) -> Self {
        Self {
            state: std::sync::Arc::new(PartialArchiveRepairState {
                committed_rows: std::sync::atomic::AtomicUsize::new(0),
                repair_claimed: std::sync::atomic::AtomicBool::new(false),
                cleanup_finished,
                cleanup_done,
                plan,
            }),
        }
    }

    fn record_committed(&self, rows: usize) {
        self.state
            .committed_rows
            .fetch_add(rows, std::sync::atomic::Ordering::Release);
    }

    fn is_partial(&self) -> bool {
        let committed_rows = self
            .state
            .committed_rows
            .load(std::sync::atomic::Ordering::Acquire);
        match &self.state.plan {
            PartialArchiveRepairPlan::Invocation { selected_group, .. } => {
                committed_rows > 0 && committed_rows < selected_group.len()
            }
            PartialArchiveRepairPlan::Timestamped { selected_group, .. } => {
                committed_rows > 0 && committed_rows < selected_group.len()
            }
        }
    }

    fn spawn_worker(&self) -> Option<tokio::task::JoinHandle<Result<()>>> {
        if !self.is_partial() {
            return None;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return None;
        };
        if self
            .state
            .repair_claimed
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .is_err()
        {
            return None;
        }
        let state = std::sync::Arc::clone(&self.state);
        Some(runtime.spawn(async move {
            while !state
                .cleanup_done
                .load(std::sync::atomic::Ordering::Acquire)
            {
                state.cleanup_finished.notified().await;
            }
            let committed_rows = state
                .committed_rows
                .load(std::sync::atomic::Ordering::Acquire);
            crate::maintenance::retention_run_cleanup_scope(async move {
                match &state.plan {
                    PartialArchiveRepairPlan::Invocation {
                        pool,
                        outcome,
                        selected_group,
                    } => {
                        repair_partial_invocation_archive(
                            pool,
                            outcome,
                            selected_group,
                            committed_rows,
                        )
                        .await
                    }
                    PartialArchiveRepairPlan::Timestamped {
                        pool,
                        outcome,
                        selected_group,
                    } => {
                        repair_partial_timestamped_archive(
                            pool,
                            outcome,
                            selected_group,
                            committed_rows,
                        )
                        .await
                    }
                }
            })
            .await
        }))
    }

    async fn finish(&self) -> Result<()> {
        let Some(worker) = self.spawn_worker() else {
            return Ok(());
        };
        worker
            .await
            .context("partial archive repair worker failed")?
    }
}

impl Drop for PartialArchiveRepairOwner {
    fn drop(&mut self) {
        if let Some(worker) = self.spawn_worker() {
            drop(worker);
        }
    }
}

#[cfg(test)]
pub(crate) async fn invocation_archive_candidate_selection_for_test(
    pool: &Pool<Sqlite>,
    cutoff: &str,
    source_max_invocation_id: Option<i64>,
    candidate_limit: usize,
) -> Result<(
    Vec<InvocationArchiveCandidate>,
    Vec<InvocationArchiveCandidate>,
)> {
    let candidates = load_invocation_archive_candidate_metadata(
        pool,
        cutoff,
        source_max_invocation_id,
        Some(candidate_limit),
    )
    .await?;
    let metadata = candidates.clone();
    let (_, selected) = select_invocation_archive_group(pool, candidates).await?;
    Ok((metadata, selected))
}

struct ArchiveSourceIdentityProof {
    resources: std::sync::Arc<tokio::sync::Mutex<Option<ArchiveSourceIdentityProofResources>>>,
    spec: ArchiveTableSpec,
    cleanup_finished: std::sync::Arc<tokio::sync::Notify>,
    cleanup_done: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

struct ArchiveSourceIdentityProofResources {
    connection: Option<SqliteConnection>,
    cleanup: Option<TempSqliteCleanup>,
    _work_owner_file: Option<crate::maintenance::archive::TaskWorkOwner>,
    proof_path: PathBuf,
    cleanup_finished: std::sync::Arc<tokio::sync::Notify>,
    cleanup_done: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for ArchiveSourceIdentityProofResources {
    fn drop(&mut self) {
        let connection = self.connection.take();
        let cleanup = self.cleanup.take();
        let work_owner_file = self._work_owner_file.take();
        let cleanup_finished = std::sync::Arc::clone(&self.cleanup_finished);
        let cleanup_done = std::sync::Arc::clone(&self.cleanup_done);
        if connection.is_none() && cleanup.is_none() && work_owner_file.is_none() {
            cleanup_done.store(true, std::sync::atomic::Ordering::Release);
            cleanup_finished.notify_waiters();
            return;
        }
        let proof_path = self.proof_path.clone();
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            // Runtime teardown cannot acknowledge native drain. Keep every ownership part
            // through process exit instead of advertising release.
            std::mem::forget(connection);
            std::mem::forget(cleanup);
            std::mem::forget(work_owner_file);
            return;
        };
        drop(handle.spawn(async move {
            let file_phase = crate::maintenance::acquire_file_phase_permit_for_cleanup().await;
            crate::maintenance::with_file_phase_permit(file_phase, async move {
                wait_for_archive_file_operation_to_finish(&proof_path).await;
                if let Some(connection) = connection {
                    let _ = connection.close().await;
                }
                drop(cleanup);
                drop(work_owner_file);
                cleanup_done.store(true, std::sync::atomic::Ordering::Release);
                cleanup_finished.notify_waiters();
            })
            .await;
        }));
    }
}

async fn wait_for_archive_file_operation_to_finish(path: &Path) {
    while crate::is_archive_file_operation_active(path) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn archive_identity_proof_load_exhausted_budget(error: &anyhow::Error) -> bool {
    retention_run_budget_expired()
        || is_retention_archive_file_operation_budget_exhausted(error)
        || error
            .to_string()
            .contains("retention archive identity proof inflate exceeded the operation deadline")
}

fn archive_identity_proof_load_timed_out(error: &anyhow::Error) -> bool {
    is_retention_archive_file_operation_budget_exhausted(error)
        || error.chain().any(|cause| {
            cause.to_string().contains(
                "retention archive identity proof inflate exceeded the operation deadline",
            )
        })
}

fn archive_identity_proof_load_cancelled(error: &anyhow::Error) -> bool {
    retention_run_shutdown_token().is_some_and(|shutdown| shutdown.is_cancelled())
        || is_retention_write_deferred(error)
        || error
            .to_string()
            .contains("retention archive file work cancelled")
}

impl ArchiveSourceIdentityProof {
    async fn load(path: &Path, spec: ArchiveTableSpec) -> Result<Self> {
        let file_phase = acquire_file_phase_permit("archive_identity_proof_load").await?;
        with_file_phase_permit(file_phase, Self::load_with_file_phase(path, spec)).await
    }

    async fn load_with_file_phase(path: &Path, spec: ArchiveTableSpec) -> Result<Self> {
        // The writer/publication path already proves the returned artifact digest. The first
        // source admission revalidates that digest under the archive fence; this local copy only
        // supplies per-chunk identity data and must not replace that final recheck.
        let proof_path = PathBuf::from(format!(
            "{}.task-{}.sqlite",
            path.display(),
            retention_temp_suffix()
        ));
        let mut work_owner_file = Some(
            crate::maintenance::archive::create_retention_identity_work_file(path, &proof_path)?,
        );
        let mut cleanup = Some(TempSqliteCleanup::new(proof_path.clone()));
        let cleanup_finished = std::sync::Arc::new(tokio::sync::Notify::new());
        let cleanup_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let result = async {
            #[cfg(test)]
            batch_admission::pause_before_archive_identity_proof_inflate().await;
            let operation_deadline = retention_archive_file_operation_deadline();
            let operation_budget = operation_deadline.saturating_duration_since(Instant::now());
            if operation_budget.is_zero() {
                return Err(retention_archive_file_operation_budget_exhausted());
            }
            if !inflate_gzip_sqlite_file_with_budget(
                path,
                &proof_path,
                Instant::now(),
                Some(operation_budget),
            )
            .await?
            {
                bail!("retention archive identity proof inflate exceeded the operation deadline");
            }
            let remaining = operation_deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(retention_archive_file_operation_budget_exhausted());
            }
            let file_phase = try_current_file_phase_permit()
                .context("archive identity proof load lost its file phase permit")?;
            open_archive_identity_connection_and_close(proof_path.clone(), file_phase, remaining)
                .await?;
            Ok(Self {
                resources: std::sync::Arc::new(tokio::sync::Mutex::new(Some(
                    ArchiveSourceIdentityProofResources {
                        connection: None,
                        cleanup: cleanup.take(),
                        _work_owner_file: work_owner_file.take(),
                        proof_path: proof_path.clone(),
                        cleanup_finished: std::sync::Arc::clone(&cleanup_finished),
                        cleanup_done: std::sync::Arc::clone(&cleanup_done),
                    },
                ))),
                spec,
                cleanup_finished,
                cleanup_done,
            })
        }
        .await;
        if result.is_err() {
            wait_for_archive_file_operation_to_finish(&proof_path).await;
            drop(cleanup.take());
            drop(work_owner_file.take());
        }
        result
    }

    async fn identity(&self, ids: &[i64]) -> Result<String> {
        let file_phase = acquire_file_phase_permit("archive_identity_proof_read").await?;
        let spec = self.spec;
        let resources = std::sync::Arc::clone(&self.resources);
        let ids = ids.to_vec();
        let operation_deadline = retention_archive_file_operation_deadline();
        let remaining = operation_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(retention_archive_file_operation_budget_exhausted());
        }
        spawn_file_phase_task(file_phase, async move {
            let mut resources_guard = resources.lock().await;
            let resources = resources_guard
                .as_mut()
                .context("archive identity proof resources are unavailable")?;
            if resources.connection.is_none() {
                let opened = tokio::time::timeout(remaining, async {
                    open_archive_sqlite_connection(&resources.proof_path).await
                })
                .await
                .map_err(|_| retention_archive_file_operation_budget_exhausted())??;
                resources.connection = Some(opened);
            }
            let result = {
                let connection = resources
                    .connection
                    .as_mut()
                    .context("archive identity proof connection is unavailable")?;
                let remaining = operation_deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    Err(retention_archive_file_operation_budget_exhausted())
                } else if spec.dataset == "codex_invocations" {
                    match tokio::time::timeout(
                        remaining,
                        invocation_archive_source_identity_sha256(
                            connection,
                            InvocationArchiveIdentityDatabase::Main,
                            &ids,
                        ),
                    )
                    .await
                    {
                        Ok(result) => result,
                        Err(_) => Err(retention_archive_file_operation_budget_exhausted()),
                    }
                } else {
                    match tokio::time::timeout(
                        remaining,
                        archive_table_source_identity_sha256(connection, spec, "main", &ids),
                    )
                    .await
                    {
                        Ok(result) => result,
                        Err(_) => Err(retention_archive_file_operation_budget_exhausted()),
                    }
                }
            };
            match result {
                Ok(result) => Ok(result),
                Err(error) => {
                    // A cancelled SQLx future may leave the connection in an unknown state. Do
                    // not carry it into another selected chunk after a proof read failed. The
                    // cleanup guard and work owner follow the connection through this drain.
                    Err(close_archive_identity_proof_resources(&mut resources_guard, error).await)
                }
            }
        })
        .await
    }

    async fn has_exact_ids(&self, ids: &[i64], expected_count: i64) -> Result<bool> {
        let file_phase = acquire_file_phase_permit("archive_identity_proof_read").await?;
        let ids_json = serde_json::to_string(ids).context("encode retention archive source ids")?;
        let resources = std::sync::Arc::clone(&self.resources);
        let operation_deadline = retention_archive_file_operation_deadline();
        let remaining = operation_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(retention_archive_file_operation_budget_exhausted());
        }
        spawn_file_phase_task(file_phase, async move {
            let mut resources_guard = resources.lock().await;
            let resources = resources_guard
                .as_mut()
                .context("archive identity proof resources are unavailable")?;
            if resources.connection.is_none() {
                let opened = tokio::time::timeout(remaining, async {
                    open_archive_sqlite_connection(&resources.proof_path).await
                })
                .await
                .map_err(|_| retention_archive_file_operation_budget_exhausted())??;
                resources.connection = Some(opened);
            }
            let result = {
                let connection = resources
                    .connection
                    .as_mut()
                    .context("archive identity proof connection is unavailable")?;
                let remaining = operation_deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    Err(retention_archive_file_operation_budget_exhausted())
                } else {
                    match tokio::time::timeout(remaining, async {
                        let archive_row_count = sqlx::query_scalar::<_, i64>(
                            "SELECT COUNT(*) FROM main.codex_invocations",
                        )
                        .fetch_one(&mut *connection)
                        .await
                        .context("count rows in archive source identity proof")?;
                        let selected_count = sqlx::query_scalar::<_, i64>(
                            "SELECT COUNT(*) FROM main.codex_invocations
                             WHERE id IN (SELECT value FROM json_each(?1))",
                        )
                        .bind(ids_json)
                        .fetch_one(&mut *connection)
                        .await
                        .context("count selected rows in archive source identity proof")?;
                        Ok::<bool, anyhow::Error>(
                            archive_row_count == expected_count && selected_count == expected_count,
                        )
                    })
                    .await
                    {
                        Ok(result) => result,
                        Err(_) => Err(retention_archive_file_operation_budget_exhausted()),
                    }
                }
            };
            match result {
                Ok(result) => Ok(result),
                Err(error) => {
                    Err(close_archive_identity_proof_resources(&mut resources_guard, error).await)
                }
            }
        })
        .await
    }

    async fn close(self) -> Result<()> {
        let file_phase = crate::maintenance::acquire_file_phase_permit_for_cleanup().await;
        let resources = std::sync::Arc::clone(&self.resources);
        spawn_file_phase_task(file_phase, async move {
            let mut resources = resources.lock().await;
            if let Some(mut resources) = resources.take() {
                close_archive_identity_proof_resources_owned(&mut resources).await?;
            }
            Ok(())
        })
        .await
    }

    fn cleanup_signal(
        &self,
    ) -> (
        std::sync::Arc<tokio::sync::Notify>,
        std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        (
            std::sync::Arc::clone(&self.cleanup_finished),
            std::sync::Arc::clone(&self.cleanup_done),
        )
    }
}

async fn close_archive_identity_proof_resources(
    resources: &mut Option<ArchiveSourceIdentityProofResources>,
    error: anyhow::Error,
) -> anyhow::Error {
    let Some(mut resources) = resources.take() else {
        return error;
    };
    if let Err(close_error) = close_archive_identity_proof_resources_owned(&mut resources).await {
        return error.context(format!(
            "close archive source identity proof after read failure: {close_error}"
        ));
    }
    drop(resources);
    error
}

async fn close_archive_identity_proof_resources_owned(
    resources: &mut ArchiveSourceIdentityProofResources,
) -> std::result::Result<(), sqlx::Error> {
    wait_for_archive_file_operation_to_finish(&resources.proof_path).await;
    let close_result = if let Some(connection) = resources.connection.take() {
        connection.close().await
    } else {
        Ok(())
    };
    drop(resources.cleanup.take());
    drop(resources._work_owner_file.take());
    resources
        .cleanup_done
        .store(true, std::sync::atomic::Ordering::Release);
    resources.cleanup_finished.notify_waiters();
    close_result
}

async fn finish_archive_identity_proof<T>(
    proof: ArchiveSourceIdentityProof,
    result: Result<T>,
) -> Result<T> {
    match (result, proof.close().await) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(close_error)) => {
            Err(close_error.context("close archive source identity proof"))
        }
        (Err(error), Err(close_error)) => Err(error.context(format!(
            "close archive source identity proof after operation failure: {close_error}"
        ))),
    }
}

async fn open_archive_identity_connection_and_close(
    proof_path: PathBuf,
    file_phase: FilePhasePermit,
    remaining: Duration,
) -> Result<()> {
    spawn_file_phase_task(file_phase, async move {
        let connection = tokio::time::timeout(remaining, async {
            open_archive_sqlite_connection(&proof_path).await
        })
        .await
        .map_err(|_| retention_archive_file_operation_budget_exhausted())??;
        connection.close().await?;
        Ok(())
    })
    .await
}

struct ReusableTaskMonthArchive {
    outcome: ArchiveBatchOutcome,
    proof: crate::VerifiedSummaryArchiveSnapshot,
    identity_proof: ArchiveSourceIdentityProof,
}

async fn reuse_task_month_archive_if_proven(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    spec: ArchiveTableSpec,
    month_key: &str,
    ids: &[i64],
    source_identity_sha256: &str,
) -> Result<Option<ReusableTaskMonthArchive>> {
    if spec.dataset != "codex_invocations" {
        return Ok(None);
    }
    let final_path = archive_batch_file_path(config, spec.dataset, month_key)?;
    if !final_path.is_file() {
        return Ok(None);
    }
    let manifest = sqlx::query_as::<
        _,
        (
            i64,
            String,
            i64,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >(
        "SELECT id, sha256, row_count, coverage_start_at, coverage_end_at, archive_expires_at
         FROM archive_batches
         WHERE dataset = ?1
           AND month_key = ?2
           AND file_path = ?3
           AND status = 'materializing'
           AND summary_source_kind = 'authoritative'
           AND sha256 IS NOT NULL
           AND TRIM(sha256) <> ''
         ORDER BY id DESC
         LIMIT 1",
    )
    .bind(spec.dataset)
    .bind(month_key)
    .bind(final_path.to_string_lossy().to_string())
    .fetch_optional(pool)
    .await?;
    let Some((
        archive_batch_id,
        sha256,
        row_count,
        coverage_start_at,
        coverage_end_at,
        archive_expires_at,
    )) = manifest
    else {
        return Ok(None);
    };

    let expected_row_count =
        i64::try_from(ids.len()).context("count retention archive source ids")?;
    if row_count != expected_row_count
        || !crate::summary_archive_snapshot_has_final_proof_read_only(
            pool,
            archive_batch_id,
            &sha256,
        )
        .await?
    {
        return Ok(None);
    }
    let manifest_coverage_matches_proof: bool = sqlx::query_scalar(
        "SELECT EXISTS(
             SELECT 1
             FROM archive_batches AS manifest
             JOIN summary_archive_snapshot_v2_proof AS proof
               ON proof.archive_batch_id = manifest.id
              AND proof.manifest_sha256 = manifest.sha256
             WHERE manifest.id = ?1 AND manifest.sha256 = ?2
               AND ((manifest.coverage_start_at IS NULL AND proof.coverage_start IS NULL)
                    OR manifest.coverage_start_at = proof.coverage_start)
               AND ((manifest.coverage_end_at IS NULL AND proof.coverage_end IS NULL)
                    OR manifest.coverage_end_at = proof.coverage_end)
         )",
    )
    .bind(archive_batch_id)
    .bind(&sha256)
    .fetch_one(pool)
    .await?;
    if !manifest_coverage_matches_proof {
        return Ok(None);
    }

    // A durable Summary proof makes the published artifact reusable. The source admission
    // below still rechecks the file digest while holding the archive fence before deletion.
    let identity_proof = match ArchiveSourceIdentityProof::load(&final_path, spec).await {
        Ok(proof) => proof,
        Err(_) => return Ok(None),
    };
    let has_exact_ids = match identity_proof.has_exact_ids(ids, expected_row_count).await {
        Ok(has_exact_ids) => has_exact_ids,
        Err(_) => {
            let _ = identity_proof.close().await;
            return Ok(None);
        }
    };
    if !has_exact_ids {
        let _ = identity_proof.close().await;
        return Ok(None);
    }
    let archive_identity = match identity_proof.identity(ids).await {
        Ok(identity) => identity,
        Err(_) => {
            let _ = identity_proof.close().await;
            return Ok(None);
        }
    };
    if archive_identity != source_identity_sha256 {
        let _ = identity_proof.close().await;
        return Ok(None);
    }
    let proof =
        match crate::prepare_verified_summary_archive_snapshot(pool, archive_batch_id, &sha256)
            .await
        {
            Ok(proof) => proof,
            Err(_) => {
                let _ = identity_proof.close().await;
                return Ok(None);
            }
        };
    Ok(Some(ReusableTaskMonthArchive {
        outcome: ArchiveBatchOutcome {
            dataset: spec.dataset,
            month_key: month_key.to_string(),
            day_key: None,
            part_key: None,
            file_path: final_path.to_string_lossy().to_string(),
            sha256,
            source_identity_sha256: Some(source_identity_sha256.to_string()),
            row_count,
            upstream_last_activity: Vec::new(),
            coverage_start_at,
            coverage_end_at,
            archive_expires_at,
            summary_source_kind: SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE,
            layout: ARCHIVE_LAYOUT_LEGACY_MONTH,
            codec: ARCHIVE_FILE_CODEC_GZIP,
            writer_version: ARCHIVE_WRITER_VERSION_LEGACY_MONTH_V1,
            cleanup_state: ARCHIVE_CLEANUP_STATE_ACTIVE,
            superseded_by: None,
        },
        proof,
        identity_proof,
    }))
}

struct RetentionBatchCommit {
    raw_deletion_plan: ProxyRawDeletionPlan,
    raw_reference_check_elapsed: Option<Duration>,
    commit_started: Instant,
    prompt_cache_keys: Vec<String>,
}

type InvocationDetailProofResult = (Option<(usize, usize, usize)>, usize);
type InvocationArchiveProofResult = (bool, usize);

struct RetentionSqliteTransaction<'a>(&'a mut SqliteConnection);

impl RetentionSqliteTransaction<'_> {
    fn as_mut(&mut self) -> &mut SqliteConnection {
        self.0
    }
}

fn take_retention_batch<T>(
    pending: &mut Vec<T>,
    config: &AppConfig,
    operation: &'static str,
    estimated_bytes: impl Fn(&T) -> usize,
) -> Vec<T> {
    let candidate_count = pending
        .len()
        .min(retention_micro_batch_limit(config, operation));
    let indices = (0..candidate_count).collect::<Vec<_>>();
    let selected = take_retention_micro_batch(indices, |index| estimated_bytes(&pending[*index]));
    let count = selected.len();
    pending.drain(..count).collect()
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
        let candidate_limit = archive_candidate_limit(config);
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
        let candidates = sqlx::query_as::<_, InvocationDetailPruneCandidate>(&sql)
            .bind(DETAIL_LEVEL_FULL)
            .bind(&prune_cutoff)
            .bind(&archive_cutoff)
            .bind(candidate_limit.saturating_add(1) as i64)
            .fetch_all(pool)
            .await?;
        let discovered = candidates.len();
        let has_more = discovered > candidate_limit;
        let mut candidates = candidates;
        candidates.truncate(candidate_limit);
        batch_plan::record_discovered_rows("codex_invocation_details", discovered);
        batch_plan::record_candidate_window_has_more("codex_invocation_details", has_more);
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
            count_planned_proxy_raw_paths(&raw_paths, raw_path_fallback_root),
        ));
    }

    let mut rows_pruned = 0usize;
    let mut archive_batches = 0usize;
    let mut raw_files_removed = 0usize;
    let mut recovery_diagnostic = RetentionDiagnosticRecorder::new();

    'work_unit: {
        if retention_run_budget_expired() {
            recovery_diagnostic.mark_budget();
            break 'work_unit;
        }
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
        let candidate_limit = archive_candidate_limit(config);
        let candidates_query = sqlx::query_as::<_, InvocationDetailPruneCandidate>(&sql)
            .bind(DETAIL_LEVEL_FULL)
            .bind(&prune_cutoff)
            .bind(&archive_cutoff)
            .bind(candidate_limit.saturating_add(1) as i64)
            .fetch_all(pool);
        let candidates = if let Some(remaining) = retention_run_remaining_budget() {
            match tokio::time::timeout(remaining, candidates_query).await {
                Ok(result) => result?,
                Err(_) => {
                    recovery_diagnostic.mark_budget();
                    break 'work_unit;
                }
            }
        } else {
            candidates_query.await?
        };
        let discovered = candidates.len();
        let has_more = discovered > candidate_limit;
        let mut candidates = candidates;
        candidates.truncate(candidate_limit);
        batch_plan::record_discovered_rows("codex_invocation_details", discovered);
        batch_plan::record_candidate_window_has_more("codex_invocation_details", has_more);

        if candidates.is_empty() {
            break 'work_unit;
        }

        let mut candidate_remaining_hint = has_more;
        let mut by_group: BTreeMap<String, Vec<InvocationDetailPruneCandidate>> = BTreeMap::new();
        for candidate in candidates {
            if retention_run_budget_expired() {
                recovery_diagnostic.mark_budget();
                break;
            }
            let group_key = invocation_archive_group_key(config, &candidate.occurred_at)?;
            by_group.entry(group_key).or_default().push(candidate);
        }

        for (group_key, group) in by_group {
            if retention_run_budget_expired() {
                recovery_diagnostic.mark_budget();
                break;
            }
            let group_candidate_count = group.len();
            let group = select_archive_batch(group, |candidate| {
                candidate.estimated_write_bytes.max(1) as usize
            });
            if group.len() < group_candidate_count {
                candidate_remaining_hint = true;
                batch_plan::record_candidate_window_has_more("codex_invocation_details", true);
            }
            let mut batch_observation = batch_plan::BatchObservation::begin(
                "codex_invocation_details",
                &group_key,
                group.len(),
            );
            let prepare_started = Instant::now();
            let ids = group
                .iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            if ids.is_empty() || retention_run_budget_expired() {
                if retention_run_budget_expired() {
                    recovery_diagnostic.mark_budget();
                }
                break;
            }
            let Some(mut source_connection) =
                acquire_retention_pool_connection(pool, "invocation_detail_identity").await?
            else {
                recovery_diagnostic.mark_stop();
                return Ok((rows_pruned, archive_batches, raw_files_removed));
            };
            let source_verify_phase = batch_observation.phase(RetentionPhase::SourceVerify);
            let source_identity_started = Instant::now();
            let source_identity_query = invocation_archive_source_identity_sha256(
                &mut source_connection,
                InvocationArchiveIdentityDatabase::Main,
                &ids,
            );
            let source_identity_sha256 = if let Some(remaining) = retention_run_remaining_budget() {
                match tokio::time::timeout(remaining, source_identity_query).await {
                    Ok(result) => result?,
                    Err(_) => {
                        recovery_diagnostic.mark_budget();
                        break;
                    }
                }
            } else {
                source_identity_query
                    .await
                    .context("read task-local invocation source identity")?
            };
            batch_observation.source_identity_finished(source_identity_started.elapsed());
            drop(source_connection);
            drop(source_verify_phase);
            if retention_run_budget_expired() {
                recovery_diagnostic.mark_budget();
                break;
            }
            let mut descriptor = retention_prepared_archive_descriptor(
                config,
                spec.dataset,
                &group_key,
                &load_invocation_archive_candidates_by_ids(pool, &ids).await?,
                source_identity_sha256,
                RETENTION_RECOVERY_PUBLICATION_DETAIL_PRUNE,
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
            let prepare_phase = batch_observation.phase(RetentionPhase::Prepare);
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
            let Some(mut archive_outcome) =
                (match retention_prepared_batch_or_deferred(archive_result) {
                    Ok(value) => value,
                    Err(error) => {
                        mark_error(&mut recovery_diagnostic, &error);
                        return Err(error);
                    }
                })
            else {
                recovery_diagnostic.mark_stop();
                return Ok((rows_pruned, archive_batches, raw_files_removed));
            };
            if archive_outcome.source_identity_sha256.as_deref()
                != Some(descriptor.source_identity_sha256.as_str())
            {
                recovery_diagnostic.mark_proof_unavailable();
                bail!("retention task-local archive source identity verification failed");
            }
            set_archive_batch_coverage_from_local_rows(
                &mut archive_outcome,
                group.iter().map(|candidate| candidate.occurred_at.as_str()),
                Some(config.invocation_archive_ttl_days),
            )?;
            archive_outcome.summary_source_kind = SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR;
            let pruned_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
            let prepare_elapsed = prepare_started.elapsed();
            batch_observation.file_prepared(prepare_elapsed);
            drop(prepare_phase);
            let archive_identity_proof =
                match ArchiveSourceIdentityProof::load(Path::new(&archive_outcome.file_path), spec)
                    .await
                {
                    Ok(proof) => proof,
                    Err(error) if archive_identity_proof_load_cancelled(&error) => {
                        recovery_diagnostic.mark_stop();
                        break;
                    }
                    Err(error) if archive_identity_proof_load_exhausted_budget(&error) => {
                        if archive_identity_proof_load_timed_out(&error) {
                            batch_observation.mark_timeout();
                        }
                        retention_record_defer(
                            "archive_identity_proof",
                            RETENTION_ARCHIVE_FILE_OPERATION_BUDGET_REASON,
                        );
                        recovery_diagnostic.mark_budget();
                        break;
                    }
                    Err(error) => {
                        recovery_diagnostic.mark_proof_unavailable();
                        return Err(error);
                    }
                };
            let group_result: Result<InvocationDetailProofResult> = async {
                #[cfg(test)]
                batch_admission::pause_after_archive_artifact_verified().await;
                let mut committed_rows = 0;
                let mut pending_group = group;
                while !pending_group.is_empty() {
                    let group = take_retention_batch(
                        &mut pending_group,
                        config,
                        "invocation_detail_prune",
                        |candidate| candidate.estimated_write_bytes.max(1) as usize,
                    );
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
                    let chunk_identity = match archive_identity_proof.identity(&ids).await {
                        Ok(identity) => identity,
                        Err(error) => {
                            recovery_diagnostic.mark_proof_unavailable();
                            return Err(
                                error.context("read task-local invocation source chunk identity")
                            );
                        }
                    };
                    let admission_wait_phase = batch_observation.phase(RetentionPhase::AdmissionWait);
                    let admission_result =
                        acquire_retention_batch_write_connection_with_archive_fence_observed(
                            pool,
                            "invocation_detail_prune",
                            Path::new(&archive_outcome.file_path),
                            Some(&archive_outcome.sha256),
                            || Some(batch_observation.phase(RetentionPhase::FileVerify)),
                        )
                        .await;
                    let Some((source_connection, admission, archive_lock)) = (match admission_result {
                        Ok(value) => value,
                        Err(error) => {
                            mark_error(&mut recovery_diagnostic, &error);
                            return Err(error);
                        }
                    }) else {
                        drop(admission_wait_phase);
                        recovery_diagnostic.mark_stop();
                        return Ok((
                            Some((rows_pruned, archive_batches, raw_files_removed)),
                            committed_rows,
                        ));
                    };
                    let admission_mode = admission.admission_mode();
                    let lock_wait = admission.lock_wait();
                    let p1_waiter_count = admission.p1_waiter_count();
                    let mut transaction_resources =
                        RetentionTransactionResources::new(source_connection, admission, archive_lock);
                    drop(admission_wait_phase);
                    let transaction_phase = batch_observation.phase(RetentionPhase::Transaction);
                    let execute_started = Instant::now();
                    transaction_resources.mark_transaction_started();
                    let begin_result = sqlx::query("BEGIN")
                        .execute(transaction_resources.connection_mut())
                        .await;
                    if let Err(error) = begin_result {
                        let cleanup_error = transaction_resources.rollback_and_close().await;
                        drop(transaction_phase);
                        let mut error: anyhow::Error = error.into();
                        if let Err(cleanup_error) = cleanup_error {
                            error = error.context(format!(
                                "failed to rollback and close source connection after begin failure: {cleanup_error}"
                            ));
                        }
                        return Err(error);
                    }
                    #[cfg(test)]
                    batch_admission::pause_after_transaction_begin().await;
                    let transaction_result: Result<RetentionBatchCommit> = async {
                        let mut tx = RetentionSqliteTransaction(transaction_resources.connection_mut());
                        upsert_archive_batch_manifest(tx.as_mut(), &archive_outcome).await?;
                        mark_archive_batch_historical_rollups_materialized_tx(
                            tx.as_mut(),
                            spec.dataset,
                            &archive_outcome.file_path,
                        )
                        .await?;
                        let source_verify_phase = batch_observation.phase(RetentionPhase::SourceVerify);
                        if invocation_archive_source_identity_sha256(
                            tx.as_mut(),
                            InvocationArchiveIdentityDatabase::Main,
                            &ids,
                        )
                        .await?
                            != chunk_identity
                        {
                            recovery_diagnostic.mark_proof_unavailable();
                            bail!("retention task-local detail source changed before conversion");
                        }
                        drop(source_verify_phase);
                        let prompt_cache_keys =
                            load_prompt_cache_keys_for_invocation_ids_tx(tx.as_mut(), &ids).await?;
                        let prompt_cache_key_refs = prompt_cache_keys
                            .iter()
                            .map(String::as_str)
                            .collect::<Vec<_>>();
                        if !prompt_cache_key_refs.is_empty() {
                            mark_prompt_cache_conversation_stats_stale_on_connection(tx.as_mut())
                                .await?;
                        }
                        let mut query = QueryBuilder::<Sqlite>::new(
                            "UPDATE codex_invocations SET payload = CASE WHEN json_valid(payload) AND (json_extract(payload, '$.upstreamAccountId') IS NOT NULL OR json_extract(payload, '$.requestModel') IS NOT NULL OR json_extract(payload, '$.responseModel') IS NOT NULL OR json_extract(payload, '$.reasoningEffort') IS NOT NULL OR json_extract(payload, '$.requestCompressionAlgorithm') IS NOT NULL) THEN json_patch(json_patch(json_patch(json_patch(json_patch('{}', CASE WHEN json_extract(payload, '$.upstreamAccountId') IS NOT NULL THEN json_object('upstreamAccountId', json_extract(payload, '$.upstreamAccountId')) ELSE '{}' END), CASE WHEN json_extract(payload, '$.requestModel') IS NOT NULL THEN json_object('requestModel', json_extract(payload, '$.requestModel')) ELSE '{}' END), CASE WHEN json_extract(payload, '$.responseModel') IS NOT NULL THEN json_object('responseModel', json_extract(payload, '$.responseModel')) ELSE '{}' END), CASE WHEN json_extract(payload, '$.reasoningEffort') IS NOT NULL THEN json_object('reasoningEffort', json_extract(payload, '$.reasoningEffort')) ELSE '{}' END), CASE WHEN json_extract(payload, '$.requestCompressionAlgorithm') IS NOT NULL THEN json_object('requestCompressionAlgorithm', json_extract(payload, '$.requestCompressionAlgorithm')) ELSE '{}' END) ELSE NULL END, raw_response = '', request_raw_path = NULL, request_raw_codec = 'identity', request_raw_size = NULL, request_raw_truncated = 0, request_raw_truncated_reason = NULL, response_raw_path = NULL, response_raw_codec = 'identity', response_raw_size = NULL, response_raw_truncated = 0, response_raw_truncated_reason = NULL, detail_level = ",
                        );
                        query
                            .push_bind(DETAIL_LEVEL_STRUCTURED_ONLY)
                            .push(", detail_pruned_at = ")
                            .push_bind(&pruned_at)
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
                        let delete_phase = batch_observation.phase(RetentionPhase::Delete);
                        query.build().execute(tx.as_mut()).await?;
                        drop(delete_phase);
                        // UPDATE triggers persist the affected keys. Statistics are refreshed by the
                        // materialization owner after this retention transaction commits.
                        if let Some(latest) = group
                            .iter()
                            .map(|candidate| candidate.occurred_at.as_str())
                            .max()
                        {
                            record_parallel_work_unrecoverable_detail_tx(tx.as_mut(), latest)
                                .await?;
                        }
                        let raw_reference_check_started = Instant::now();
                        let had_raw_reference_candidates = raw_paths.iter().any(Option::is_some);
                        let raw_deletion_plan = filter_unreferenced_proxy_raw_paths(
                            tx.as_mut(),
                            &raw_paths,
                            raw_path_fallback_root,
                        )
                        .await?;
                        let raw_reference_check_elapsed = had_raw_reference_candidates
                            .then(|| raw_reference_check_started.elapsed());
                        let commit_started = Instant::now();
                        Ok(RetentionBatchCommit {
                            raw_deletion_plan,
                            raw_reference_check_elapsed,
                            commit_started,
                            prompt_cache_keys: Vec::new(),
                        })
                    }
                    .await;
                    let commit = match transaction_result {
                        Ok(commit) => commit,
                        Err(error) => {
                            let cleanup_error = transaction_resources.rollback_and_close().await;
                            drop(transaction_phase);
                            let mut error = error;
                            mark_error(&mut recovery_diagnostic, &error);
                            if let Err(cleanup_error) = cleanup_error {
                                error = error.context(format!(
                                    "retention transaction rollback and close failed: {cleanup_error}"
                                ));
                            }
                            return Err(error);
                        }
                    };
                    let commit_phase = batch_observation.phase(RetentionPhase::Commit);
                    #[cfg(test)]
                    batch_admission::prepare_commit_for_test(transaction_resources.connection_mut())
                        .await;
                    let commit_started_at = Instant::now();
                    let commit_result = sqlx::query("COMMIT")
                        .execute(transaction_resources.connection_mut())
                        .await;
                    let commit_elapsed = commit_started_at.elapsed();
                    drop(commit_phase);
                    if let Err(error) = commit_result {
                        let cleanup_error = transaction_resources.rollback_and_close().await;
                        drop(transaction_phase);
                        let mut error: anyhow::Error = error.into();
                        if let Err(cleanup_error) = cleanup_error {
                            error = error.context(format!(
                                "retention transaction rollback and close failed after commit failure: {cleanup_error}"
                            ));
                        } else {
                            mark_error(&mut recovery_diagnostic, &error);
                        }
                        return Err(error);
                    }
                    transaction_resources.mark_transaction_finished();
                    let raw_deletion_plan = commit.raw_deletion_plan;
                    rows_pruned += group.len();
                    committed_rows += group.len();
                    workload::record_owner_processed_count("invocation_detail_prune", group.len());
                    recovery_diagnostic.committed(group.len());
                    batch_observation.committed(group.len(), lock_wait);
                    drop(transaction_phase);
                    let breached = transaction_resources
                        .return_to_pool_and_record_commit(RetentionWriteCommit {
                            operation: "invocation_detail_prune",
                            admission_mode,
                            rows: group.len(),
                            estimated_bytes: group
                                .iter()
                                .map(|candidate| candidate.estimated_write_bytes.max(1) as usize)
                                .sum(),
                            prepare_elapsed,
                            lock_wait,
                            execute_elapsed: commit.commit_started.duration_since(execute_started),
                            commit_elapsed,
                            write_hold_elapsed: None,
                            raw_reference_check_elapsed: commit.raw_reference_check_elapsed,
                            p1_waiter_count,
                            candidate_remaining_hint: usize::from(candidate_remaining_hint),
                        })
                        .await
                        .context("failed to return retention transaction connection to pool")?;
                    let cleanup_phase = batch_observation.phase(RetentionPhase::Cleanup);
                    if !raw_deletion_plan.paths.is_empty() {
                        mark_retention_raw_inventory_reset_intent(pool).await?;
                    }
                    let removed = delete_proxy_raw_paths(raw_deletion_plan, raw_path_fallback_root)?;
                    raw_files_removed += removed;
                    workload::record_owner_raw_files_removed("invocation_detail_prune", removed);
                    batch_observation.raw_files_removed(removed);
                    drop(cleanup_phase);
                    if breached {
                        tokio::task::yield_now().await;
                    }
                }
                Ok((None, committed_rows))
            }
            .await;
            let (early_return, committed_rows) =
                finish_archive_identity_proof(archive_identity_proof, group_result).await?;
            if let Some(result) = early_return {
                return Ok(result);
            }
            if committed_rows > 0 {
                archive_batches += 1;
            }
            batch_observation.complete();
        }
        // One invocation owns the selected bounded archive window. A test-injected operation
        // deadline may stop it inside that window; production scheduling does not use a total
        // timer to split the work into hidden follow-up tasks.
        break 'work_unit;
    }

    recovery_diagnostic.finish();
    Ok((rows_pruned, archive_batches, raw_files_removed))
}

#[cfg(test)]
pub(crate) async fn archive_old_invocations(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
) -> Result<(usize, usize, usize, std::collections::HashSet<String>)> {
    let cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    archive_old_invocations_with_source_max(
        pool,
        config,
        raw_path_fallback_root,
        dry_run,
        &cutoff,
        None,
    )
    .await
}

pub(super) async fn archive_old_invocations_with_source_max(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
    cutoff: &str,
    source_max_invocation_id: Option<i64>,
) -> Result<(usize, usize, usize, std::collections::HashSet<String>)> {
    let spec = archive_table_spec("codex_invocations");

    if dry_run {
        let candidate_limit = archive_candidate_limit(config);
        let candidates_query = load_invocation_archive_candidate_metadata(
            pool,
            cutoff,
            source_max_invocation_id,
            Some(candidate_limit.saturating_add(1)),
        );
        let candidates = if let Some(remaining) = retention_run_remaining_budget() {
            match tokio::time::timeout(remaining, candidates_query).await {
                Ok(result) => result?,
                Err(_) => {
                    bail!(
                        "retention invocation archive candidate query exceeded the operation deadline"
                    )
                }
            }
        } else {
            candidates_query.await?
        };
        let discovered = candidates.len();
        let has_more = discovered > candidate_limit;
        let mut candidates = candidates;
        candidates.truncate(candidate_limit);
        batch_plan::record_discovered_rows(spec.dataset, discovered);
        batch_plan::record_candidate_window_has_more(spec.dataset, has_more);

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
            count_planned_proxy_raw_paths(&raw_paths, raw_path_fallback_root),
            std::collections::HashSet::new(),
        ));
    }

    let mut rows_archived = 0usize;
    let mut archive_batches = 0usize;
    let mut raw_files_removed = 0usize;
    let mut prompt_cache_keys = std::collections::HashSet::new();
    let mut discovered_ids = HashSet::new();
    let mut recovery_diagnostic = RetentionDiagnosticRecorder::new();

    'work_unit: {
        if retention_run_budget_expired() {
            recovery_diagnostic.mark_budget();
            break 'work_unit;
        }
        if !archive_batch_can_start() {
            recovery_diagnostic.mark_budget();
            break 'work_unit;
        }
        let candidate_limit = archive_candidate_limit(config);
        let candidates_query = load_invocation_archive_candidate_metadata(
            pool,
            cutoff,
            source_max_invocation_id,
            Some(candidate_limit.saturating_add(1)),
        );
        let candidates = if let Some(remaining) = retention_run_remaining_budget() {
            match tokio::time::timeout(remaining, candidates_query).await {
                Ok(result) => result?,
                Err(_) => {
                    recovery_diagnostic.mark_budget();
                    break 'work_unit;
                }
            }
        } else {
            candidates_query.await?
        };
        let discovered = candidates.len();
        let has_more = discovered > candidate_limit;
        let mut candidates = candidates;
        candidates.truncate(candidate_limit);
        batch_plan::record_discovered_rows(spec.dataset, discovered);
        batch_plan::record_candidate_window_has_more(spec.dataset, has_more);

        if candidates.is_empty() {
            break 'work_unit;
        }

        discovered_ids.extend(candidates.iter().map(|candidate| candidate.id));
        workload::record_discovered_count(discovered_ids.len());

        let mut candidate_remaining_hint = has_more;
        let mut by_group: BTreeMap<String, Vec<InvocationArchiveCandidate>> = BTreeMap::new();
        for candidate in candidates {
            if retention_run_budget_expired() {
                recovery_diagnostic.mark_budget();
                break;
            }
            let group_key = invocation_archive_group_key(config, &candidate.occurred_at)?;
            by_group.entry(group_key).or_default().push(candidate);
        }

        for (group_key, group) in by_group {
            if retention_run_budget_expired() {
                recovery_diagnostic.mark_budget();
                break;
            }
            let group_candidate_count = group.len();
            let selection = select_invocation_archive_group(pool, group).await;
            let (ids, group) = match selection {
                Ok(selection) => selection,
                Err(error) => {
                    if error
                        .to_string()
                        .contains("archive source selection changed")
                    {
                        recovery_diagnostic.mark_proof_unavailable();
                    } else {
                        mark_error(&mut recovery_diagnostic, &error);
                    }
                    return Err(error);
                }
            };
            if group.len() < group_candidate_count {
                candidate_remaining_hint = true;
                batch_plan::record_candidate_window_has_more(spec.dataset, true);
            }
            if ids.is_empty() || retention_run_budget_expired() {
                if retention_run_budget_expired() {
                    recovery_diagnostic.mark_budget();
                }
                break;
            }
            let mut batch_observation =
                batch_plan::BatchObservation::begin(spec.dataset, &group_key, group.len());
            let Some(mut source_connection) =
                acquire_retention_pool_connection(pool, "invocation_archive_identity").await?
            else {
                recovery_diagnostic.mark_stop();
                return Ok((
                    rows_archived,
                    archive_batches,
                    raw_files_removed,
                    prompt_cache_keys,
                ));
            };
            let source_verify_phase = batch_observation.phase(RetentionPhase::SourceVerify);
            let source_identity_started = Instant::now();
            let source_identity_query = invocation_archive_source_identity_sha256(
                &mut source_connection,
                InvocationArchiveIdentityDatabase::Main,
                &ids,
            );
            let source_identity_sha256 = if let Some(remaining) = retention_run_remaining_budget() {
                match tokio::time::timeout(remaining, source_identity_query).await {
                    Ok(result) => result?,
                    Err(_) => {
                        batch_observation
                            .source_identity_finished(source_identity_started.elapsed());
                        recovery_diagnostic.mark_budget();
                        break;
                    }
                }
            } else {
                source_identity_query
                    .await
                    .context("read task-local source identity")?
            };
            batch_observation.source_identity_finished(source_identity_started.elapsed());
            drop(source_connection);
            drop(source_verify_phase);
            if retention_run_budget_expired() {
                recovery_diagnostic.mark_budget();
                break;
            }
            let prepare_started = Instant::now();
            let prepare_phase = batch_observation.phase(RetentionPhase::Prepare);
            let archive_layout = archive_layout_for_dataset(config, spec.dataset);
            let reusable_archive = if archive_layout == ArchiveBatchLayout::LegacyMonth {
                let reuse_future = reuse_task_month_archive_if_proven(
                    pool,
                    config,
                    spec,
                    &group_key,
                    &ids,
                    &source_identity_sha256,
                );
                if let Some(remaining) = retention_run_remaining_budget() {
                    match tokio::time::timeout(remaining, reuse_future).await {
                        Ok(result) => result?,
                        Err(_) => {
                            recovery_diagnostic.mark_budget();
                            break;
                        }
                    }
                } else {
                    reuse_future.await?
                }
            } else {
                None
            };
            let (archive_outcome, proof, archive_identity_proof, prepare_elapsed) =
                if let Some(reusable_archive) = reusable_archive {
                    let elapsed = prepare_started.elapsed();
                    batch_observation.archive_writer_finished(elapsed);
                    batch_observation.file_prepared(elapsed);
                    batch_observation.summary_proof_finished(elapsed);
                    drop(prepare_phase);
                    (
                        reusable_archive.outcome,
                        reusable_archive.proof,
                        reusable_archive.identity_proof,
                        elapsed,
                    )
                } else {
                    let mut snapshot_pages = if archive_layout == ArchiveBatchLayout::SegmentV1 {
                        invocation_archive_snapshot_pages(&group)?
                    } else {
                        Vec::new()
                    };
                    let archive_writer_started = Instant::now();
                    let archive_result = {
                        let archive_future = async {
                            match archive_layout {
                                ArchiveBatchLayout::LegacyMonth => {
                                    archive_rows_into_task_month_batch(
                                        pool,
                                        config,
                                        spec,
                                        &group_key,
                                        &ids,
                                        &mut snapshot_pages,
                                    )
                                    .await
                                }
                                ArchiveBatchLayout::SegmentV1 => {
                                    archive_rows_into_segment_batch(
                                        pool, config, spec, &group_key, &ids,
                                    )
                                    .await
                                }
                            }
                        };
                        tokio::pin!(archive_future);
                        if let Some(remaining) = retention_run_remaining_budget() {
                            match tokio::time::timeout(remaining, &mut archive_future).await {
                                Ok(result) => result,
                                Err(_) => {
                                    batch_observation
                                        .archive_writer_finished(archive_writer_started.elapsed());
                                    recovery_diagnostic.mark_budget();
                                    break;
                                }
                            }
                        } else {
                            archive_future.await
                        }
                    };
                    batch_observation.archive_writer_finished(archive_writer_started.elapsed());
                    let Some(mut archive_outcome) = (match archive_result {
                        Ok(outcome) => Some(outcome),
                        Err(error)
                            if is_retention_archive_file_operation_budget_exhausted(&error) =>
                        {
                            recovery_diagnostic.mark_budget();
                            return Err(error);
                        }
                        Err(error) if is_retention_write_deferred(&error) => {
                            retention_recovery_record_deferred("preparing");
                            recovery_diagnostic.mark_stop();
                            return Ok((
                                rows_archived,
                                archive_batches,
                                raw_files_removed,
                                prompt_cache_keys,
                            ));
                        }
                        Err(error) => {
                            mark_error(&mut recovery_diagnostic, &error);
                            return Err(error.context("prepare task-local invocation month file"));
                        }
                    }) else {
                        recovery_diagnostic.mark_stop();
                        return Ok((
                            rows_archived,
                            archive_batches,
                            raw_files_removed,
                            prompt_cache_keys,
                        ));
                    };
                    if archive_outcome.source_identity_sha256.as_deref()
                        != Some(source_identity_sha256.as_str())
                    {
                        recovery_diagnostic.mark_proof_unavailable();
                        bail!("retention task-local archive source identity verification failed");
                    }
                    set_archive_batch_coverage_from_local_rows(
                        &mut archive_outcome,
                        group.iter().map(|candidate| candidate.occurred_at.as_str()),
                        None,
                    )?;
                    archive_outcome.coverage_start_at = snapshot_pages
                        .first()
                        .map(|page| page.coverage_start.clone());
                    archive_outcome.coverage_end_at =
                        snapshot_pages.last().map(|page| page.coverage_end.clone());
                    archive_outcome.archive_expires_at =
                        Some(shanghai_archive_expiry_from_reference_timestamp(
                            &format_utc_iso(Utc::now()),
                            config.invocation_archive_ttl_days,
                        )?);
                    archive_outcome.summary_source_kind = SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE;
                    if archive_layout == ArchiveBatchLayout::SegmentV1
                        && !persist_invocation_segment_summary_pages(
                            pool,
                            &archive_outcome,
                            &snapshot_pages,
                        )
                        .await?
                    {
                        recovery_diagnostic.mark_stop();
                        break;
                    }
                    let prepare_elapsed = prepare_started.elapsed();
                    batch_observation.file_prepared(prepare_elapsed);
                    drop(prepare_phase);
                    let archive_identity_proof = match ArchiveSourceIdentityProof::load(
                        Path::new(&archive_outcome.file_path),
                        spec,
                    )
                    .await
                    {
                        Ok(proof) => proof,
                        Err(error) if archive_identity_proof_load_cancelled(&error) => {
                            recovery_diagnostic.mark_stop();
                            break;
                        }
                        Err(error) if archive_identity_proof_load_exhausted_budget(&error) => {
                            if archive_identity_proof_load_timed_out(&error) {
                                batch_observation.mark_timeout();
                            }
                            retention_record_defer(
                                "archive_identity_proof",
                                RETENTION_ARCHIVE_FILE_OPERATION_BUDGET_REASON,
                            );
                            recovery_diagnostic.mark_budget();
                            break;
                        }
                        Err(error) => {
                            recovery_diagnostic.mark_proof_unavailable();
                            return Err(error);
                        }
                    };
                    let summary_proof_started = Instant::now();
                    let proof_future = batch_plan::prepare_summary_proof(pool, &archive_outcome);
                    let proof_result = if let Some(remaining) = retention_run_remaining_budget() {
                        match tokio::time::timeout(remaining, proof_future).await {
                            Ok(result) => {
                                result.context("prepare task-local monthly Summary proof")
                            }
                            Err(_) => {
                                batch_observation
                                    .summary_proof_finished(summary_proof_started.elapsed());
                                recovery_diagnostic.mark_budget();
                                let _ = archive_identity_proof.close().await;
                                break;
                            }
                        }
                    } else {
                        proof_future
                            .await
                            .context("prepare task-local monthly Summary proof")
                    };
                    batch_observation.summary_proof_finished(summary_proof_started.elapsed());
                    let proof = match proof_result {
                        Ok(Some(proof)) => proof,
                        Ok(None) => {
                            recovery_diagnostic.mark_proof_unavailable();
                            let _ = archive_identity_proof.close().await;
                            return Ok((
                                rows_archived,
                                archive_batches,
                                raw_files_removed,
                                prompt_cache_keys,
                            ));
                        }
                        Err(error) => {
                            recovery_diagnostic.mark_proof_unavailable();
                            let _ = archive_identity_proof.close().await;
                            return Err(error);
                        }
                    };
                    (
                        archive_outcome,
                        proof,
                        archive_identity_proof,
                        prepare_elapsed,
                    )
                };
            let selected_group = group;
            let repair_owner = PartialArchiveRepairOwner::invocation(
                pool,
                &archive_outcome,
                &selected_group,
                archive_identity_proof.cleanup_signal(),
            );
            let mut committed_rows = 0;
            let proof_result: Result<InvocationArchiveProofResult> = async {
                #[cfg(test)]
                batch_admission::pause_after_archive_artifact_verified().await;
                let mut chunk_index = 0usize;
                let mut pending_group = selected_group.clone();
                while !pending_group.is_empty() {
                let group = take_retention_batch(
                    &mut pending_group,
                    config,
                    "invocation_archive",
                    |candidate| {
                        candidate
                            .payload
                            .as_deref()
                            .map_or(256, |payload| payload.len())
                    },
                );
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
                let materialized_rows = group
                    .iter()
                    .map(invocation_archive_candidate_to_hourly_source_record)
                    .collect::<Vec<_>>();
                let chunk_identity = match archive_identity_proof.identity(&ids).await {
                    Ok(identity) => identity,
                    Err(error) => {
                        recovery_diagnostic.mark_proof_unavailable();
                        return Err(
                            error.context("read task-local invocation source chunk identity")
                        );
                    }
                };
                let admission_wait_phase = batch_observation.phase(RetentionPhase::AdmissionWait);
                let admission_result =
                    acquire_retention_batch_write_connection_with_archive_fence_observed(
                        pool,
                        "invocation_archive",
                        Path::new(&archive_outcome.file_path),
                        Some(&archive_outcome.sha256),
                        || Some(batch_observation.phase(RetentionPhase::FileVerify)),
                    )
                    .await;
                    let Some((source_connection, admission, archive_lock)) = (match admission_result {
                    Ok(value) => value,
                    Err(error) => {
                        mark_error(&mut recovery_diagnostic, &error);
                        return Err(error);
                    }
                    }) else {
                        drop(admission_wait_phase);
                        recovery_diagnostic.mark_stop();
                        return Ok((true, committed_rows));
                    };
                    let admission_mode = admission.admission_mode();
                    let lock_wait = admission.lock_wait();
                    let p1_waiter_count = admission.p1_waiter_count();
                    let mut transaction_resources =
                        RetentionTransactionResources::new(source_connection, admission, archive_lock);
                    #[cfg(test)]
                    transaction_resources.observe_return_for_test("invocation_archive");
                    drop(admission_wait_phase);
                    let transaction_phase = batch_observation.phase(RetentionPhase::Transaction);
                    let execute_started = Instant::now();
                    transaction_resources.mark_transaction_started();
                    let begin_result = sqlx::query("BEGIN")
                        .execute(transaction_resources.connection_mut())
                        .await;
                    if let Err(error) = begin_result {
                        let cleanup_error = transaction_resources.rollback_and_close().await;
                        drop(transaction_phase);
                        let mut error: anyhow::Error = error.into();
                        if let Err(cleanup_error) = cleanup_error {
                            error = error.context(format!(
                                "failed to rollback and close source connection after begin failure: {cleanup_error}"
                            ));
                        }
                        return Err(error);
                    }
                    #[cfg(test)]
                    batch_admission::pause_after_transaction_begin().await;
                    let transaction_result: Result<RetentionBatchCommit> = async {
                    let mut tx = RetentionSqliteTransaction(transaction_resources.connection_mut());
                    // P2 normally advances this cursor before retention. Rows beyond it would be
                    // deleted before the regular replay can observe them, so materialize just those
                    // rows in this same archive transaction before claiming the archive is covered.
                    let live_rollup_cursor = load_hourly_rollup_live_progress_tx(
                        tx.as_mut(),
                        HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    )
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
                        let unprojected_ids = unprojected_rows
                            .iter()
                            .map(|row| row.id)
                            .collect::<Vec<_>>();
                        // Stop at the first uncovered ID instead of counting a million-row prefix
                        // while holding the writer permit when old timestamps have newer IDs.
                        let has_uncovered_prefix: bool = sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM codex_invocations WHERE id > ?1 AND id <= ?2
                         AND id NOT IN (SELECT value FROM json_each(?3)))",
                    )
                    .bind(live_rollup_cursor)
                    .bind(prefix_end)
                    .bind(serde_json::to_string(&unprojected_ids)?)
                    .fetch_one(tx.as_mut())
                    .await?;
                        if !has_uncovered_prefix {
                            save_hourly_rollup_live_progress_tx(
                                tx.as_mut(),
                                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                                prefix_end,
                            )
                            .await?;
                        }
                    }
                    upsert_invocation_rollups(tx.as_mut(), &group).await?;
                    // The file fence is released between chunks. Recheck pages and manifest in
                    // the source transaction after any admission wait or concurrent rewrite.
                    crate::validate_verified_summary_archive_snapshot_tx(tx.as_mut(), &proof)
                        .await?;
                    if chunk_index == 0 {
                        mark_archive_batch_historical_rollups_materialized_tx(
                            tx.as_mut(),
                            spec.dataset,
                            &archive_outcome.file_path,
                        )
                        .await?;
                    }
                    let source_verify_phase = batch_observation.phase(RetentionPhase::SourceVerify);
                    if invocation_archive_source_identity_sha256(
                        tx.as_mut(),
                        InvocationArchiveIdentityDatabase::Main,
                        &ids,
                    )
                    .await?
                        != chunk_identity
                    {
                        recovery_diagnostic.mark_proof_unavailable();
                        bail!("retention task-local archive source changed before conversion");
                    }
                    drop(source_verify_phase);
                    let ids_json = serde_json::to_string(&ids)?;
                    let archived_prompt_cache_keys = sqlx::query_scalar::<_, String>(&format!(
                    "SELECT DISTINCT {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} AS prompt_cache_key \
                 FROM codex_invocations \
                 WHERE id IN (SELECT value FROM json_each(?1)) \
                   AND {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} IS NOT NULL \
                   AND {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} <> ''"
                ))
                .bind(ids_json)
                .fetch_all(tx.as_mut())
                .await?;
                    let archived_prompt_cache_key_refs = archived_prompt_cache_keys
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>();
                    if !archived_prompt_cache_key_refs.is_empty() {
                        mark_prompt_cache_conversation_stats_stale_on_connection(tx.as_mut())
                            .await?;
                    }
                    let delete_phase = batch_observation.phase(RetentionPhase::Delete);
                    delete_rows_by_ids(tx.as_mut(), spec.dataset, &ids).await?;
                    drop(delete_phase);
                    // DELETE triggers persist the affected keys. Statistics are refreshed asynchronously
                    // by the materialization owner after archive publication.
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
                    if chunk_index == 0 {
                        finalize_invocation_archive_batch_publication_tx(
                            tx.as_mut(),
                            &archive_outcome.file_path,
                        )
                        .await?;
                        // Manifest publication invalidates the pre-publication proof. Store the
                        // final marker only after that state transition has completed.
                        store_verified_summary_archive_snapshot_tx(tx.as_mut(), &proof).await?;
                    }
                    let raw_reference_check_started = Instant::now();
                    let had_raw_reference_candidates = raw_paths.iter().any(Option::is_some);
                    let raw_deletion_plan = filter_unreferenced_proxy_raw_paths(
                        tx.as_mut(),
                        &raw_paths,
                        raw_path_fallback_root,
                    )
                    .await?;
                    let raw_reference_check_elapsed =
                        had_raw_reference_candidates.then(|| raw_reference_check_started.elapsed());
                    let commit_started = Instant::now();
                    Ok(RetentionBatchCommit {
                        raw_deletion_plan,
                        raw_reference_check_elapsed,
                        commit_started,
                        prompt_cache_keys: archived_prompt_cache_keys,
                    })
                    }
                    .await;
                    let commit = match transaction_result {
                    Ok(commit) => commit,
                    Err(error) => {
                        let cleanup_error = transaction_resources.rollback_and_close().await;
                        drop(transaction_phase);
                        let mut error = error;
                        mark_error(&mut recovery_diagnostic, &error);
                        if let Err(cleanup_error) = cleanup_error {
                            error = error.context(format!(
                                "retention transaction rollback and close failed: {cleanup_error}"
                            ));
                        }
                        return Err(error);
                    }
                    };
                    let commit_phase = batch_observation.phase(RetentionPhase::Commit);
                    #[cfg(test)]
                    batch_admission::prepare_commit_for_test(transaction_resources.connection_mut())
                        .await;
                    let commit_started_at = Instant::now();
                    let commit_result = sqlx::query("COMMIT")
                        .execute(transaction_resources.connection_mut())
                        .await;
                    let commit_elapsed = commit_started_at.elapsed();
                    drop(commit_phase);
                    if let Err(error) = commit_result {
                        let cleanup_error = transaction_resources.rollback_and_close().await;
                        drop(transaction_phase);
                        let mut error: anyhow::Error = error.into();
                        if let Err(cleanup_error) = cleanup_error {
                            error = error.context(format!(
                                "retention transaction rollback and close failed after commit failure: {cleanup_error}"
                            ));
                        } else {
                            mark_error(&mut recovery_diagnostic, &error);
                        }
                        return Err(error);
                    }
                    transaction_resources.mark_transaction_finished();
                    prompt_cache_keys.extend(commit.prompt_cache_keys);
                    let raw_deletion_plan = commit.raw_deletion_plan;
                    workload::record_processed_count(group.len());
                    rows_archived += group.len();
                    committed_rows += group.len();
                    repair_owner.record_committed(group.len());
                    recovery_diagnostic.committed(group.len());
                    batch_observation.committed(group.len(), lock_wait);
                    drop(transaction_phase);
                    let breached = transaction_resources
                    .return_to_pool_and_record_commit(RetentionWriteCommit {
                        operation: "invocation_archive",
                        admission_mode,
                        rows: group.len(),
                        estimated_bytes: group
                            .iter()
                            .map(|candidate| {
                                candidate
                                    .payload
                                    .as_deref()
                                    .map_or(256, |payload| payload.len())
                            })
                            .sum(),
                        prepare_elapsed,
                        lock_wait,
                        execute_elapsed: commit.commit_started.duration_since(execute_started),
                        commit_elapsed,
                        write_hold_elapsed: None,
                        raw_reference_check_elapsed: commit.raw_reference_check_elapsed,
                        p1_waiter_count,
                        candidate_remaining_hint: usize::from(candidate_remaining_hint),
                    })
                    .await
                        .context("failed to return retention transaction connection to pool")?;
                    let cleanup_phase = batch_observation.phase(RetentionPhase::Cleanup);
                    if !raw_deletion_plan.paths.is_empty() {
                        mark_retention_raw_inventory_reset_intent(pool).await?;
                    }
                    retention_recovery_record_progress();
                    let removed = delete_proxy_raw_paths(raw_deletion_plan, raw_path_fallback_root)?;
                    raw_files_removed += removed;
                    workload::record_owner_raw_files_removed("retention_archive", removed);
                    batch_observation.raw_files_removed(removed);
                    drop(cleanup_phase);
                    if breached {
                        tokio::task::yield_now().await;
                    }
                    #[cfg(test)]
                    batch_admission::pause_after_first_chunk(committed_rows).await;
                    chunk_index += 1;
                }
                Ok((false, committed_rows))
            }
            .await;
            let proof_close_result = archive_identity_proof.close().await;
            let repair_result = repair_owner
                .finish()
                .await
                .context("repair partially committed invocation archive");
            let proof_result = match (proof_result, proof_close_result) {
                (Ok(value), Ok(())) => Ok(value),
                (Err(error), Ok(())) => Err(error),
                (Ok(_), Err(close_error)) => {
                    Err(close_error.context("close archive source identity proof"))
                }
                (Err(error), Err(close_error)) => Err(error.context(format!(
                    "close archive source identity proof after operation failure: {close_error}"
                ))),
            };
            if let Err(repair_error) = repair_result {
                return Err(match proof_result {
                    Ok(_) => repair_error,
                    Err(error) => repair_error
                        .context(format!("partial archive operation also failed: {error}")),
                });
            }
            let (early_return, committed_rows) = proof_result?;
            if early_return {
                return Ok((
                    rows_archived,
                    archive_batches,
                    raw_files_removed,
                    prompt_cache_keys,
                ));
            }
            if committed_rows > 0 {
                archive_batches += 1;
            }
            batch_observation.complete();
        }
        // One scheduled invocation owns one selected candidate window. It completes that logical
        // work unit before returning; the next invocation reselects the remaining live rows.
        break 'work_unit;
    }

    recovery_diagnostic.finish();
    Ok((
        rows_archived,
        archive_batches,
        raw_files_removed,
        prompt_cache_keys,
    ))
}

pub(crate) async fn archive_timestamped_dataset(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    spec: ArchiveTableSpec,
    select_sql: &str,
    cutoff: String,
    dry_run: bool,
    raw_path_fallback_root: Option<&Path>,
) -> Result<(usize, usize, usize)> {
    if dry_run {
        let candidate_limit = archive_candidate_limit(config);
        let candidates_query = sqlx::query_as::<_, TimestampedArchiveCandidate>(select_sql)
            .bind(&cutoff)
            .bind(candidate_limit.saturating_add(1) as i64)
            .fetch_all(pool);
        let candidates = if let Some(remaining) = retention_run_remaining_budget() {
            match tokio::time::timeout(remaining, candidates_query).await {
                Ok(result) => result?,
                Err(_) => {
                    bail!(
                        "retention timestamped archive candidate query exceeded the operation deadline"
                    )
                }
            }
        } else {
            candidates_query.await?
        };
        let discovered = candidates.len();
        let has_more = discovered > candidate_limit;
        let candidates = candidates
            .into_iter()
            .take(candidate_limit)
            .collect::<Vec<_>>();
        batch_plan::record_discovered_rows(spec.dataset, discovered);
        batch_plan::record_candidate_window_has_more(spec.dataset, has_more);
        let raw_files_planned = if spec.dataset == "pool_upstream_request_attempts" {
            let ids = candidates
                .iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            let raw_paths = sqlx::query_scalar::<_, Option<String>>(
                "SELECT response_raw_path
                 FROM pool_upstream_request_attempts
                 WHERE id IN (SELECT value FROM json_each(?1))",
            )
            .bind(serde_json::to_string(&ids)?)
            .fetch_all(pool)
            .await?;
            count_planned_proxy_raw_paths(&raw_paths, raw_path_fallback_root)
        } else {
            0
        };
        let mut by_month: BTreeMap<String, usize> = BTreeMap::new();
        for candidate in candidates {
            let month_key =
                archive_timestamped_dataset_month_key(spec.dataset, &candidate.timestamp_value)?;
            *by_month.entry(month_key).or_default() += 1;
        }
        for (month_key, rows) in &by_month {
            info!(
                dataset = spec.dataset,
                month_key = %month_key,
                rows = *rows,
                "retention dry-run planned archive batch"
            );
        }
        return Ok((by_month.values().sum(), by_month.len(), raw_files_planned));
    }

    let mut rows_archived = 0usize;
    let mut archive_batches = 0usize;
    let mut raw_files_removed = 0usize;
    let mut recovery_diagnostic = RetentionDiagnosticRecorder::new();

    'work_unit: {
        if retention_run_budget_expired() {
            recovery_diagnostic.mark_budget();
            break 'work_unit;
        }
        if !archive_batch_can_start() {
            recovery_diagnostic.mark_budget();
            break 'work_unit;
        }
        let candidate_limit = archive_candidate_limit(config);
        let candidates_query = sqlx::query_as::<_, TimestampedArchiveCandidate>(select_sql)
            .bind(&cutoff)
            .bind(candidate_limit.saturating_add(1) as i64)
            .fetch_all(pool);
        let candidates = if let Some(remaining) = retention_run_remaining_budget() {
            match tokio::time::timeout(remaining, candidates_query).await {
                Ok(result) => result?,
                Err(_) => {
                    recovery_diagnostic.mark_budget();
                    break 'work_unit;
                }
            }
        } else {
            candidates_query.await?
        };
        let discovered = candidates.len();
        let has_more = discovered > candidate_limit;
        let mut candidates = candidates;
        candidates.truncate(candidate_limit);
        batch_plan::record_discovered_rows(spec.dataset, discovered);
        batch_plan::record_candidate_window_has_more(spec.dataset, has_more);

        if candidates.is_empty() {
            break 'work_unit;
        }

        let mut candidate_remaining_hint = has_more;
        let mut by_month: BTreeMap<String, Vec<TimestampedArchiveCandidate>> = BTreeMap::new();
        for candidate in candidates {
            if retention_run_budget_expired() {
                recovery_diagnostic.mark_budget();
                break;
            }
            let month_key =
                archive_timestamped_dataset_month_key(spec.dataset, &candidate.timestamp_value)?;
            by_month.entry(month_key).or_default().push(candidate);
        }

        for (month_key, group) in by_month {
            if retention_run_budget_expired() {
                recovery_diagnostic.mark_budget();
                break;
            }
            let candidate_ids = group.iter().map(|row| row.id).collect::<Vec<_>>();
            let row_sizes = archive_source_row_sizes(pool, spec, &candidate_ids).await?;
            let group_candidate_count = group.len();
            let group = select_archive_batch(group, |candidate| {
                row_sizes.get(&candidate.id).copied().unwrap_or(512)
            });
            if group.len() < group_candidate_count {
                candidate_remaining_hint = true;
                batch_plan::record_candidate_window_has_more(spec.dataset, true);
            }
            let mut batch_observation =
                batch_plan::BatchObservation::begin(spec.dataset, &month_key, group.len());
            let prepare_started = Instant::now();
            let ids = group
                .iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            if ids.is_empty() || retention_run_budget_expired() {
                if retention_run_budget_expired() {
                    recovery_diagnostic.mark_budget();
                }
                break;
            }
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
            let Some(mut source_connection) =
                acquire_retention_pool_connection(pool, "timestamped_archive_identity").await?
            else {
                recovery_diagnostic.mark_stop();
                return Ok((rows_archived, archive_batches, raw_files_removed));
            };
            let source_verify_phase = batch_observation.phase(RetentionPhase::SourceVerify);
            let source_identity_started = Instant::now();
            let source_identity_query =
                archive_table_source_identity_sha256(&mut source_connection, spec, "main", &ids);
            let source_identity = if let Some(remaining) = retention_run_remaining_budget() {
                match tokio::time::timeout(remaining, source_identity_query).await {
                    Ok(result) => result?,
                    Err(_) => {
                        batch_observation
                            .source_identity_finished(source_identity_started.elapsed());
                        recovery_diagnostic.mark_budget();
                        break;
                    }
                }
            } else {
                source_identity_query.await?
            };
            batch_observation.source_identity_finished(source_identity_started.elapsed());
            drop(source_connection);
            drop(source_verify_phase);
            let prepare_phase = batch_observation.phase(RetentionPhase::Prepare);
            let archive_writer_started = Instant::now();
            let archive_result = {
                let archive_future =
                    archive_rows_into_month_batch(pool, config, spec, &month_key, &ids);
                tokio::pin!(archive_future);
                if let Some(remaining) = retention_run_remaining_budget() {
                    match tokio::time::timeout(remaining, &mut archive_future).await {
                        Ok(result) => result,
                        Err(_) => {
                            batch_observation
                                .archive_writer_finished(archive_writer_started.elapsed());
                            recovery_diagnostic.mark_budget();
                            break;
                        }
                    }
                } else {
                    archive_future.await
                }
            };
            batch_observation.archive_writer_finished(archive_writer_started.elapsed());
            let Some(mut archive_outcome) =
                (match retention_prepared_batch_or_deferred(archive_result) {
                    Ok(value) => value,
                    Err(error) => {
                        mark_error(&mut recovery_diagnostic, &error);
                        return Err(error);
                    }
                })
            else {
                recovery_diagnostic.mark_stop();
                return Ok((rows_archived, archive_batches, raw_files_removed));
            };
            if archive_outcome.source_identity_sha256.as_deref() != Some(source_identity.as_str()) {
                recovery_diagnostic.mark_proof_unavailable();
                bail!("timestamped task-local archive source identity verification failed");
            }
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
            batch_observation.file_prepared(prepare_elapsed);
            drop(prepare_phase);
            let archive_identity_proof =
                match ArchiveSourceIdentityProof::load(Path::new(&archive_outcome.file_path), spec)
                    .await
                {
                    Ok(proof) => proof,
                    Err(error) if archive_identity_proof_load_cancelled(&error) => {
                        recovery_diagnostic.mark_stop();
                        break;
                    }
                    Err(error) if archive_identity_proof_load_exhausted_budget(&error) => {
                        if archive_identity_proof_load_timed_out(&error) {
                            batch_observation.mark_timeout();
                        }
                        retention_record_defer(
                            "archive_identity_proof",
                            RETENTION_ARCHIVE_FILE_OPERATION_BUDGET_REASON,
                        );
                        recovery_diagnostic.mark_budget();
                        break;
                    }
                    Err(error) => {
                        recovery_diagnostic.mark_proof_unavailable();
                        return Err(error);
                    }
                };
            #[cfg(test)]
            batch_admission::pause_after_archive_artifact_verified().await;
            let archive_file_contains_only_new_rows = archive_outcome.row_count == ids.len() as i64;
            let mut committed_rows = 0;
            let selected_group = group;
            let repair_owner = PartialArchiveRepairOwner::timestamped(
                pool,
                &archive_outcome,
                &selected_group,
                archive_identity_proof.cleanup_signal(),
            );
            let mut pending_group = selected_group.clone();
            let operation_result: Result<()> = async {
                while !pending_group.is_empty() {
                let group = take_retention_batch(
                    &mut pending_group,
                    config,
                    "timestamped_archive",
                    |candidate| row_sizes.get(&candidate.id).copied().unwrap_or(512),
                );
                let ids = group
                    .iter()
                    .map(|candidate| candidate.id)
                    .collect::<Vec<_>>();
                let pool_attempt_raw_paths = if spec.dataset == "pool_upstream_request_attempts" {
                    sqlx::query_scalar::<_, Option<String>>("SELECT response_raw_path FROM pool_upstream_request_attempts WHERE id IN (SELECT value FROM json_each(?1))")
                        .bind(serde_json::to_string(&ids)?).fetch_all(pool).await?
                } else {
                    Vec::new()
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
                let chunk_identity = match archive_identity_proof.identity(&ids).await {
                    Ok(identity) => identity,
                    Err(error) => {
                        recovery_diagnostic.mark_proof_unavailable();
                        return Err(
                            error.context("read task-local timestamped archive chunk identity")
                        );
                    }
                };
                let admission_wait_phase = batch_observation.phase(RetentionPhase::AdmissionWait);
                let admission_result =
                    acquire_retention_batch_write_connection_with_archive_fence_observed(
                        pool,
                        "timestamped_archive",
                        Path::new(&archive_outcome.file_path),
                        Some(&archive_outcome.sha256),
                        || Some(batch_observation.phase(RetentionPhase::FileVerify)),
                    )
                    .await;
                let Some((source_connection, admission, archive_lock)) = (match admission_result {
                    Ok(value) => value,
                    Err(error) => {
                        mark_error(&mut recovery_diagnostic, &error);
                        return Err(error);
                    }
                }) else {
                    drop(admission_wait_phase);
                    recovery_diagnostic.mark_stop();
                    break;
                };
                let admission_mode = admission.admission_mode();
                let lock_wait = admission.lock_wait();
                let p1_waiter_count = admission.p1_waiter_count();
                let mut transaction_resources =
                    RetentionTransactionResources::new(source_connection, admission, archive_lock);
                drop(admission_wait_phase);
                let transaction_phase = batch_observation.phase(RetentionPhase::Transaction);
                let execute_started = Instant::now();
                transaction_resources.mark_transaction_started();
                let begin_result = sqlx::query("BEGIN")
                    .execute(transaction_resources.connection_mut())
                    .await;
                if let Err(error) = begin_result {
                    let cleanup_error = transaction_resources.rollback_and_close().await;
                    drop(transaction_phase);
                    let mut error: anyhow::Error = error.into();
                    if let Err(cleanup_error) = cleanup_error {
                        error = error.context(format!(
                            "failed to rollback and close source connection after begin failure: {cleanup_error}"
                        ));
                    }
                    return Err(error);
                }
                #[cfg(test)]
                batch_admission::pause_after_transaction_begin().await;
                let transaction_result: Result<Option<RetentionBatchCommit>> = async {
                    let mut tx = RetentionSqliteTransaction(transaction_resources.connection_mut());
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
                    return Ok(None);
                }
                let source_verify_phase = batch_observation.phase(RetentionPhase::SourceVerify);
                if archive_table_source_identity_sha256(tx.as_mut(), spec, "main", &ids).await?
                    != chunk_identity
                {
                    recovery_diagnostic.mark_proof_unavailable();
                    bail!("timestamped task-local archive source changed before conversion");
                }
                drop(source_verify_phase);
                upsert_archive_batch_manifest(tx.as_mut(), &archive_outcome).await?;
                if spec.dataset == "pool_upstream_request_attempts" {
                    let archive_batch_id = load_archive_batch_id_for_file_tx(
                        tx.as_mut(),
                        spec.dataset,
                        &archive_outcome.month_key,
                        &archive_outcome.file_path,
                    )
                    .await?;
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
                    cache_pool_node_health_batch_chunk_tx(
                        tx.as_mut(),
                        archive_batch_id,
                        &archive_outcome.file_path,
                        &ids,
                        committed_rows == 0,
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
                let delete_phase = batch_observation.phase(RetentionPhase::Delete);
                delete_rows_by_ids(tx.as_mut(), spec.dataset, &ids).await?;
                drop(delete_phase);
                mark_retention_archived_hourly_rollup_targets_tx(
                    tx.as_mut(),
                    spec.dataset,
                    &[],
                    &materialized_forward_proxy_rows,
                )
                .await?;
                let (raw_deletion_plan, raw_reference_check_elapsed) = if spec.dataset
                    == "pool_upstream_request_attempts"
                {
                    let raw_reference_check_started = Instant::now();
                    let had_raw_reference_candidates =
                        pool_attempt_raw_paths.iter().any(Option::is_some);
                    let raw_deletion_plan = filter_unreferenced_proxy_raw_paths(
                        tx.as_mut(),
                        &pool_attempt_raw_paths,
                        config.database_path.parent(),
                    )
                    .await?;
                    (
                        raw_deletion_plan,
                        had_raw_reference_candidates.then(|| raw_reference_check_started.elapsed()),
                    )
                } else {
                    (ProxyRawDeletionPlan::empty(), None)
                };
                let commit_started = Instant::now();
                Ok(Some(RetentionBatchCommit {
                    raw_deletion_plan,
                    raw_reference_check_elapsed,
                    commit_started,
                    prompt_cache_keys: Vec::new(),
                }))
                }
                .await;
                let commit = match transaction_result {
                    Ok(commit) => commit,
                    Err(error) => {
                        let cleanup_error = transaction_resources.rollback_and_close().await;
                        drop(transaction_phase);
                        let mut error = error;
                        mark_error(&mut recovery_diagnostic, &error);
                        if let Err(cleanup_error) = cleanup_error {
                            error = error.context(format!(
                                "retention transaction rollback and close failed: {cleanup_error}"
                            ));
                        }
                        return Err(error);
                    }
                };
                let commit_phase = batch_observation.phase(RetentionPhase::Commit);
                #[cfg(test)]
                batch_admission::prepare_commit_for_test(transaction_resources.connection_mut())
                    .await;
                let commit_started_at = Instant::now();
                let commit_result = sqlx::query("COMMIT")
                    .execute(transaction_resources.connection_mut())
                    .await;
                let commit_elapsed = commit_started_at.elapsed();
                drop(commit_phase);
                if let Err(error) = commit_result {
                    let cleanup_error = transaction_resources.rollback_and_close().await;
                    drop(transaction_phase);
                    let mut error: anyhow::Error = error.into();
                    if let Err(cleanup_error) = cleanup_error {
                        error = error.context(format!(
                            "retention transaction rollback and close failed after commit failure: {cleanup_error}"
                        ));
                    } else {
                        mark_error(&mut recovery_diagnostic, &error);
                    }
                    return Err(error);
                }
                transaction_resources.mark_transaction_finished();
                let Some(commit) = commit else {
                    drop(transaction_phase);
                    let cleanup_error = transaction_resources.return_to_pool().await;
                    if let Err(cleanup_error) = cleanup_error {
                        return Err(anyhow!(
                            "failed to return retention transaction connection to pool: {cleanup_error}"
                        ));
                    }
                    recovery_diagnostic.mark_stop();
                    break;
                };
                let raw_deletion_plan = commit.raw_deletion_plan;
                workload::record_owner_processed_count(
                    if spec.dataset == "pool_upstream_request_attempts" {
                        "pool_upstream_attempt_archive"
                    } else {
                        "forward_proxy_attempt_archive"
                    },
                    group.len(),
                );
                rows_archived += group.len();
                committed_rows += group.len();
                repair_owner.record_committed(group.len());
                recovery_diagnostic.committed(group.len());
                batch_observation.committed(group.len(), lock_wait);
                drop(transaction_phase);
                let breached = transaction_resources
                    .return_to_pool_and_record_commit(RetentionWriteCommit {
                        operation: "timestamped_archive",
                        admission_mode,
                        rows: group.len(),
                        estimated_bytes: group.len().saturating_mul(256),
                        prepare_elapsed,
                        lock_wait,
                        execute_elapsed: commit.commit_started.duration_since(execute_started),
                        commit_elapsed,
                        write_hold_elapsed: None,
                        raw_reference_check_elapsed: commit.raw_reference_check_elapsed,
                        p1_waiter_count,
                        candidate_remaining_hint: usize::from(candidate_remaining_hint),
                    })
                    .await
                    .context("failed to return retention transaction connection to pool")?;
                let cleanup_phase = batch_observation.phase(RetentionPhase::Cleanup);
                if spec.dataset == "pool_upstream_request_attempts" {
                    if !raw_deletion_plan.paths.is_empty() {
                        mark_retention_raw_inventory_reset_intent(pool).await?;
                    }
                    let removed =
                        delete_proxy_raw_paths(raw_deletion_plan, config.database_path.parent())?;
                    raw_files_removed += removed;
                    workload::record_owner_raw_files_removed(
                        "pool_upstream_attempt_archive",
                        removed,
                    );
                    batch_observation.raw_files_removed(removed);
                }
                drop(cleanup_phase);
                if breached {
                    tokio::task::yield_now().await;
                }
                #[cfg(test)]
                batch_admission::pause_after_first_chunk(committed_rows).await;
                }
                Ok(())
            }
            .await;
            let operation_result =
                finish_archive_identity_proof(archive_identity_proof, operation_result).await;
            let repair_result = repair_owner
                .finish()
                .await
                .context("repair partially committed timestamped archive");
            if let Err(repair_error) = repair_result {
                return Err(match operation_result {
                    Ok(()) => repair_error,
                    Err(operation_error) => repair_error.context(format!(
                        "timestamped archive operation failed before partial repair: {operation_error}"
                    )),
                });
            }
            operation_result?;
            if committed_rows > 0 {
                archive_batches += 1;
            }
            batch_observation.complete();
        }
        // Keep the selected timestamped candidate window as one logical work unit. Operation
        // budgets and cancellation may stop it earlier, but it is never split by a total timer.
        break 'work_unit;
    }

    recovery_diagnostic.finish();
    Ok((rows_archived, archive_batches, raw_files_removed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn retention_error_classifier_distinguishes_lock_from_unclassified_failure() {
        let lock_runtime = crate::ObservabilityRuntime::new(true);
        crate::observability::contention::with_performance(lock_runtime.clone(), async {
            let mut diagnostic = RetentionDiagnosticRecorder::new();
            mark_error(&mut diagnostic, &anyhow::anyhow!("database is locked"));
            drop(diagnostic);
        })
        .await;
        let lock_metrics = lock_runtime.render();
        assert!(
            lock_metrics.lines().any(|line| {
                line.starts_with("cvm_runtime_recovery_diagnostics_total{")
                    && line.contains("operation=\"retention\"")
                    && line.contains("reason=\"pressure_deferred\"")
                    && line.ends_with(" 1")
            }),
            "known SQLite locks must remain pressure-deferred: {lock_metrics}"
        );
        assert!(!lock_metrics.lines().any(|line| {
            line.starts_with("cvm_runtime_recovery_diagnostics_total{")
                && line.contains("operation=\"retention\"")
                && line.contains("reason=\"failed\"")
                && line.ends_with(" 1")
        }));

        let failure_runtime = crate::ObservabilityRuntime::new(true);
        crate::observability::contention::with_performance(failure_runtime.clone(), async {
            let mut diagnostic = RetentionDiagnosticRecorder::new();
            mark_error(&mut diagnostic, &anyhow::anyhow!("commit failed: injected"));
            drop(diagnostic);
        })
        .await;
        let failure_metrics = failure_runtime.render();
        assert!(
            failure_metrics.lines().any(|line| {
                line.starts_with("cvm_runtime_recovery_diagnostics_total{")
                    && line.contains("operation=\"retention\"")
                    && line.contains("reason=\"failed\"")
                    && line.ends_with(" 1")
            }),
            "unclassified commit failures must remain failed: {failure_metrics}"
        );
    }
}
