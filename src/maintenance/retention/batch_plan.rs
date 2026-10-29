use super::observability::{RetentionBatchObservation, RetentionPhaseGuard};
use super::*;
use crate::observability::contention::{Outcome, RetentionOperation, RetentionPhase};
use std::collections::BTreeMap;

const ARCHIVE_BATCH_TARGET_ROWS: usize = 1_000;
const ARCHIVE_BATCH_MAX_BYTES: usize = 16 * 1024 * 1024;
const ARCHIVE_BATCH_SETTLEMENT_RESERVE: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub(crate) struct TaskArchiveSnapshotPage {
    pub(crate) coverage_start: String,
    pub(crate) coverage_end: String,
    pub(crate) row_count: u32,
    pub(crate) payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskArchiveSnapshotPageFingerprint {
    pub(crate) page_index: i64,
    pub(crate) coverage_start: String,
    pub(crate) coverage_end: String,
    pub(crate) row_count: i64,
    pub(crate) payload_bytes: i64,
    pub(crate) payload_sha256: String,
    pub(crate) format_version: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RetentionBatchMetrics {
    pub(crate) dataset: String,
    pub(crate) month_key: String,
    pub(crate) batch_rows: usize,
    pub(crate) committed_rows: usize,
    pub(crate) raw_files_removed: usize,
    pub(crate) file_prepare_ms: Option<u64>,
    pub(crate) lock_wait_ms: u64,
    pub(crate) elapsed_ms: u64,
    pub(crate) timeout_count: u64,
    pub(crate) committed_rows_per_second: Option<f64>,
    pub(crate) arrival_rows_per_second: Option<f64>,
    pub(crate) service_rate_multiple: Option<f64>,
    pub(crate) small_batch_reason: Option<String>,
    pub(crate) source_identity_ms: Option<u64>,
    pub(crate) archive_writer_ms: Option<u64>,
    pub(crate) summary_proof_ms: Option<u64>,
}

tokio::task_local! {
    pub(super) static TASK_BATCH_METRICS: RefCell<Vec<RetentionBatchMetrics>>;
    pub(super) static TASK_DISCOVERED_ROWS: RefCell<BTreeMap<String, usize>>;
    pub(super) static TASK_CANDIDATE_WINDOWS: RefCell<BTreeMap<String, bool>>;
    pub(super) static TASK_RETRY_WINDOW_REMAINING: RefCell<Option<usize>>;
}

pub(crate) fn record_discovered_rows(dataset: &str, rows: usize) {
    let _ = TASK_DISCOVERED_ROWS.try_with(|discovered| {
        let mut discovered = discovered.borrow_mut();
        let entry = discovered.entry(dataset.to_string()).or_default();
        *entry = (*entry).saturating_add(rows);
    });
}

pub(crate) fn discovered_rows(dataset: &str) -> Option<usize> {
    TASK_DISCOVERED_ROWS
        .try_with(|discovered| discovered.borrow().get(dataset).copied())
        .ok()
        .flatten()
}

pub(crate) fn record_candidate_window_has_more(dataset: &str, has_more: bool) {
    let _ = TASK_CANDIDATE_WINDOWS.try_with(|windows| {
        let mut windows = windows.borrow_mut();
        let entry = windows.entry(dataset.to_string()).or_default();
        *entry |= has_more;
    });
}

pub(crate) fn candidate_window_has_more(dataset: &str) -> Option<bool> {
    TASK_CANDIDATE_WINDOWS
        .try_with(|windows| windows.borrow().get(dataset).copied())
        .ok()
        .flatten()
}

/// Remove the uncommitted tail of a batch before retrying it in the same owner
/// execution. Keep the remaining selected-window budget so a retry cannot
/// silently select a fresh window after committing only a prefix.
pub(super) fn retain_committed_batch_prefixes_for_retry() {
    let committed_rows = TASK_BATCH_METRICS
        .try_with(|metrics| {
            metrics
                .borrow()
                .iter()
                .map(|batch| batch.committed_rows)
                .sum::<usize>()
        })
        .unwrap_or_default();
    let _ = TASK_RETRY_WINDOW_REMAINING.try_with(|remaining| {
        *remaining.borrow_mut() = Some(ARCHIVE_BATCH_TARGET_ROWS.saturating_sub(committed_rows));
    });
    let _ = TASK_BATCH_METRICS.try_with(|metrics| {
        metrics.borrow_mut().retain_mut(|batch| {
            if batch.committed_rows == 0 {
                return false;
            }
            if batch.committed_rows < batch.batch_rows {
                batch.batch_rows = batch.committed_rows;
                batch.small_batch_reason = Some("internal_pressure_retry".to_string());
            }
            true
        });
    });
}

pub(crate) struct BatchObservation {
    metrics: RetentionBatchMetrics,
    started: Instant,
    retention: RetentionBatchObservation,
}

impl BatchObservation {
    pub(crate) fn begin(dataset: &str, month: &str, rows: usize) -> Self {
        Self {
            started: Instant::now(),
            retention: RetentionBatchObservation::new(match dataset {
                "codex_invocations" => RetentionOperation::InvocationArchive,
                "codex_invocation_details" => RetentionOperation::DetailPrune,
                _ => RetentionOperation::TimestampedArchive,
            }),
            metrics: RetentionBatchMetrics {
                dataset: dataset.to_string(),
                month_key: month.to_string(),
                batch_rows: rows,
                committed_rows: 0,
                raw_files_removed: 0,
                file_prepare_ms: None,
                lock_wait_ms: 0,
                elapsed_ms: 0,
                timeout_count: 0,
                committed_rows_per_second: None,
                arrival_rows_per_second: None,
                service_rate_multiple: None,
                small_batch_reason: (rows < 512)
                    .then(|| "tail_config_or_payload_limit".to_string()),
                source_identity_ms: None,
                archive_writer_ms: None,
                summary_proof_ms: None,
            },
        }
    }
    pub(crate) fn file_prepared(&mut self, elapsed: Duration) {
        self.metrics.file_prepare_ms = Some(elapsed.as_millis() as u64);
    }
    pub(crate) fn committed(&mut self, rows: usize, lock_wait: Duration) {
        self.metrics.committed_rows += rows;
        self.retention.committed(rows);
        self.metrics.lock_wait_ms = self
            .metrics
            .lock_wait_ms
            .saturating_add(lock_wait.as_millis() as u64);
    }
    pub(crate) fn raw_files_removed(&mut self, count: usize) {
        self.metrics.raw_files_removed = self.metrics.raw_files_removed.saturating_add(count);
    }
    pub(crate) fn source_identity_finished(&mut self, elapsed: Duration) {
        self.metrics.source_identity_ms = Some(elapsed.as_millis() as u64);
    }
    pub(crate) fn archive_writer_finished(&mut self, elapsed: Duration) {
        self.metrics.archive_writer_ms = Some(elapsed.as_millis() as u64);
    }
    pub(crate) fn summary_proof_finished(&mut self, elapsed: Duration) {
        self.metrics.summary_proof_ms = Some(elapsed.as_millis() as u64);
    }
    pub(crate) fn mark_timeout(&mut self) {
        self.metrics.timeout_count = self.metrics.timeout_count.max(1);
    }

    pub(crate) fn phase(&self, phase: RetentionPhase) -> RetentionPhaseGuard {
        self.retention.phase(phase)
    }

    pub(crate) fn finish(&mut self, outcome: Outcome) {
        self.retention.finish(outcome);
    }

    pub(crate) fn complete(&mut self) {
        if self.metrics.committed_rows < self.metrics.batch_rows
            && (retention_run_budget_expired()
                || retention_run_defer_reason().is_some()
                || retention_run_shutdown_token().is_some_and(|shutdown| shutdown.is_cancelled()))
        {
            return;
        }
        self.finish(Outcome::Success);
    }
}

impl Drop for BatchObservation {
    fn drop(&mut self) {
        // Attribute the run deadline only to an unfinished selected batch. A batch
        // committed before settlement is not a timeout merely because Drop is late.
        self.metrics.timeout_count = self.metrics.timeout_count.max(u64::from(
            self.metrics.committed_rows < self.metrics.batch_rows && retention_run_budget_expired(),
        ));
        self.metrics.elapsed_ms = self.started.elapsed().as_millis() as u64;
        if !self.retention.has_outcome() {
            let outcome = if super::retention_run_shutdown_token()
                .is_some_and(|shutdown| shutdown.is_cancelled())
            {
                Outcome::Cancelled
            } else if super::retention_run_budget_expired()
                || super::retention_run_defer_reason().is_some()
            {
                Outcome::Deferred
            } else {
                Outcome::Error
            };
            self.retention.finish(outcome);
        }
        let _ =
            TASK_BATCH_METRICS.try_with(|metrics| metrics.borrow_mut().push(self.metrics.clone()));
    }
}

#[cfg(test)]
pub(crate) async fn retention_test_with_batch_metrics<F: std::future::Future>(
    work: F,
) -> (F::Output, Vec<RetentionBatchMetrics>) {
    TASK_BATCH_METRICS
        .scope(RefCell::new(Vec::new()), async {
            let output = work.await;
            let metrics = TASK_BATCH_METRICS.with(|metrics| metrics.borrow().clone());
            (output, metrics)
        })
        .await
}

pub(super) fn collect_run_metrics(summary: &mut RetentionRunSummary) {
    summary.batches = TASK_BATCH_METRICS.with(|metrics| metrics.borrow().clone());
    for (dataset, counter) in [
        ("codex_invocations", &mut summary.invocation_rows_archived),
        (
            "pool_upstream_request_attempts",
            &mut summary.pool_upstream_request_attempt_rows_archived,
        ),
        (
            "forward_proxy_attempts",
            &mut summary.forward_proxy_attempt_rows_archived,
        ),
        (
            "codex_invocation_details",
            &mut summary.invocation_details_pruned,
        ),
        (
            "codex_quota_snapshots",
            &mut summary.quota_snapshot_rows_archived,
        ),
    ] {
        *counter = (*counter).max(
            summary
                .batches
                .iter()
                .filter(|batch| batch.dataset == dataset)
                .map(|batch| batch.committed_rows)
                .sum(),
        );
    }
    summary.archive_batches_touched = summary.archive_batches_touched.max(
        summary
            .batches
            .iter()
            .filter(|batch| batch.committed_rows > 0)
            .count(),
    );
}

/// File work is amortized across a selected batch, independently of the SQL write limiter.
/// A run selects once per dataset; another run selects the remaining live rows afresh.
pub(crate) fn archive_candidate_limit(_config: &AppConfig) -> usize {
    TASK_RETRY_WINDOW_REMAINING
        .try_with(|remaining| {
            remaining
                .borrow()
                .map_or(ARCHIVE_BATCH_TARGET_ROWS, |remaining| {
                    remaining.min(ARCHIVE_BATCH_TARGET_ROWS)
                })
        })
        .unwrap_or(ARCHIVE_BATCH_TARGET_ROWS)
}

pub(crate) fn select_archive_batch<T>(rows: Vec<T>, bytes: impl Fn(&T) -> usize) -> Vec<T> {
    let mut selected = Vec::with_capacity(rows.len().min(ARCHIVE_BATCH_TARGET_ROWS));
    let mut total_bytes = 0usize;
    for row in rows {
        let row_bytes = bytes(&row).max(1);
        if !selected.is_empty()
            && (selected.len() >= ARCHIVE_BATCH_TARGET_ROWS
                || total_bytes.saturating_add(row_bytes) > ARCHIVE_BATCH_MAX_BYTES)
        {
            break;
        }
        total_bytes = total_bytes.saturating_add(row_bytes);
        selected.push(row);
    }
    selected
}

pub(crate) async fn archive_source_row_sizes(
    pool: &Pool<Sqlite>,
    spec: ArchiveTableSpec,
    ids: &[i64],
) -> Result<HashMap<i64, usize>> {
    let sum = spec
        .columns
        .split(", ")
        .map(|column| format!("COALESCE(length({column}), 0)"))
        .collect::<Vec<_>>()
        .join(" + ");
    Ok(sqlx::query_as::<_, (i64, i64)>(&format!(
        "SELECT id, 512 + {sum} FROM {} WHERE id IN (SELECT value FROM json_each(?1))",
        spec.dataset,
    ))
    .bind(serde_json::to_string(ids)?)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(id, bytes)| (id, bytes.max(1) as usize))
    .collect())
}

pub(crate) fn archive_batch_can_start() -> bool {
    retention_run_remaining_budget()
        .is_none_or(|remaining| remaining > ARCHIVE_BATCH_SETTLEMENT_RESERVE)
        && !retention_run_budget_expired()
}

pub(crate) fn archive_sqlite_size_from_header(header: &[u8; 100]) -> Result<u64> {
    if &header[..16] != b"SQLite format 3\0" {
        bail!("archive disk preflight requires a SQLite database header");
    }
    let encoded_page_size = u16::from_be_bytes([header[16], header[17]]);
    let page_size = if encoded_page_size == 1 {
        65_536u64
    } else {
        u64::from(encoded_page_size)
    };
    if !(512..=65_536).contains(&page_size) || !page_size.is_power_of_two() {
        bail!("archive disk preflight found an invalid SQLite page size");
    }
    let page_count = u32::from_be_bytes(header[28..32].try_into()?);
    // SQLite only considers the in-header page count valid when these counters match.
    // Do not substitute gzip ISIZE: it wraps modulo 2^32 for large monthly files.
    if page_count == 0 || header[24..28] != header[92..96] {
        bail!("archive disk preflight cannot establish the full SQLite file size");
    }
    Ok(page_size * u64::from(page_count))
}

fn archive_inflated_bytes(path: &Path) -> Result<u64> {
    use std::io::{Read, Seek, SeekFrom};
    let file = File::open(path)?;
    let mut header = [0; 100];
    flate2::read::GzDecoder::new(file)
        .read_exact(&mut header)
        .context("failed to read archive SQLite size metadata")?;
    let inflated_bytes = archive_sqlite_size_from_header(&header)?;
    let mut file = File::open(path)?;
    file.seek(SeekFrom::End(-4))?;
    let mut footer = [0; 4];
    file.read_exact(&mut footer)?;
    if u32::from_le_bytes(footer) != inflated_bytes as u32 {
        bail!("archive disk preflight found inconsistent SQLite and gzip sizes");
    }
    Ok(inflated_bytes)
}

pub(crate) async fn archive_file_can_start(
    pool: &Pool<Sqlite>,
    spec: ArchiveTableSpec,
    path: &Path,
    ids: &[i64],
) -> Result<bool> {
    let sum = spec
        .columns
        .split(", ")
        .map(|column| format!("COALESCE(length({column}), 0)"))
        .collect::<Vec<_>>()
        .join(" + ");
    let source_bytes = sqlx::query_scalar::<_, Option<i64>>(&format!(
        "SELECT SUM(512 + {sum}) FROM {} WHERE id IN (SELECT value FROM json_each(?1))",
        spec.dataset,
    ))
    .bind(serde_json::to_string(ids)?)
    .fetch_one(pool)
    .await?
    .unwrap_or(0)
    .max(0) as u64;
    let (compressed_bytes, inflated_bytes) = match fs::metadata(path) {
        Ok(metadata) => (metadata.len(), archive_inflated_bytes(path)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (0, 0),
        Err(error) => return Err(error.into()),
    };
    // Reserve a working database and SQLite scratch space, in addition to gzip publication.
    let required = inflated_bytes
        .saturating_mul(2)
        .saturating_add(source_bytes.saturating_mul(2))
        .saturating_add(compressed_bytes.saturating_mul(2))
        .saturating_add(64 * 1024 * 1024);
    if crate::filesystem_available_bytes(path).is_some_and(|available| available < required) {
        retention_record_defer("archive_batch_planning", "archive_disk_space");
        return Ok(false);
    }
    // Conservative cold estimate; the cost does not shrink the selected file batch to single
    // rows. If the file cannot fit, finish this run with an explicit design-budget defer.
    let estimated_ms = compressed_bytes
        .saturating_add(source_bytes)
        .saturating_mul(1_000)
        .saturating_div(4 * 1024 * 1024)
        .saturating_add((ids.len() as u64).saturating_mul(2))
        .saturating_add(5_000);
    if retention_run_remaining_budget()
        .is_some_and(|remaining| remaining < Duration::from_millis(estimated_ms))
    {
        retention_record_defer("archive_batch_planning", "archive_design_budget");
        return Ok(false);
    }
    Ok(true)
}

pub(super) async fn arrival_rates(pool: &Pool<Sqlite>) -> HashMap<String, f64> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let Some(mut connection) = tokio::time::timeout(
        Duration::from_secs(2),
        SqliteConnection::connect_with(pool.connect_options().as_ref()),
    )
    .await
    .ok()
    .and_then(Result::ok) else {
        return HashMap::new();
    };
    let result = async {
        sqlx::query("PRAGMA busy_timeout=2000")
            .execute(&mut connection)
            .await?;
        {
            let mut handle = connection.lock_handle().await?;
            handle.set_progress_handler(1_000, move || Instant::now() < deadline);
        }
        let now = Utc::now().with_timezone(&Shanghai).naive_local();
        let start = format_naive(now - chrono::Duration::hours(24));
        let end = format_naive(now);
        let mut rates = HashMap::new();
        for dataset in ["codex_invocations", "pool_upstream_request_attempts"] {
            let count = sqlx::query_scalar::<_, i64>(&format!(
                "SELECT COUNT(*) FROM {dataset} WHERE occurred_at >= ?1 AND occurred_at <= ?2",
            ))
            .bind(&start)
            .bind(&end)
            .fetch_one(&mut connection)
            .await?;
            rates.insert(dataset.to_string(), count.max(0) as f64 / 86_400.0);
        }
        Ok::<_, anyhow::Error>(rates)
    }
    .await;
    // This dedicated connection is never returned to the pool, including error/cancel exits.
    let _ = connection.close().await;
    result.unwrap_or_default()
}

pub(crate) async fn stage_task_summary_archive_manifest(
    pool: &Pool<Sqlite>,
    outcome: &ArchiveBatchOutcome,
    expected_existing_sha256: Option<&str>,
    expected_existing_row_count: Option<i64>,
) -> Result<Option<i64>> {
    let Some((connection, admission, archive_lock)) =
        acquire_retention_batch_write_connection_with_archive_fence(
            pool,
            "archive_snapshot_manifest",
            Path::new(&outcome.file_path),
            expected_existing_sha256,
        )
        .await?
    else {
        return Ok(None);
    };
    let mut resources = RetentionTransactionResources::new(connection, admission, archive_lock);
    resources.mark_transaction_started();
    if let Err(error) = sqlx::query("BEGIN")
        .execute(resources.connection_mut())
        .await
    {
        let cleanup_error = resources.rollback_and_close().await;
        let mut error: anyhow::Error = error.into();
        if let Err(cleanup_error) = cleanup_error {
            error = error.context(format!(
                "failed to rollback and close initial summary manifest transaction: {cleanup_error}"
            ));
        }
        return Err(error);
    }
    let staging_outcome = if let Some(existing_sha256) = expected_existing_sha256 {
        // A legacy monthly file can predate its manifest. Stage the predecessor identity in the
        // first row as well as on conflict, so cancellation cannot leave old bytes paired with
        // the replacement SHA.
        let mut staging_outcome = outcome.clone();
        staging_outcome.sha256 = existing_sha256.to_string();
        staging_outcome.row_count = expected_existing_row_count
            .context("existing archive digest requires its predecessor row count")?;
        staging_outcome
    } else {
        outcome.clone()
    };
    let batch_id = match stage_invocation_archive_batch_manifest_for_summary_pages(
        resources.connection_mut(),
        &staging_outcome,
    )
    .await
    {
        Ok(batch_id) => batch_id,
        Err(error) => {
            let cleanup_error = resources.rollback_and_close().await;
            let mut error = error;
            if let Err(cleanup_error) = cleanup_error {
                error = error.context(format!(
                    "initial summary manifest rollback and close failed: {cleanup_error}"
                ));
            }
            return Err(error);
        }
    };
    if let Err(error) = sqlx::query("COMMIT")
        .execute(resources.connection_mut())
        .await
    {
        let cleanup_error = resources.rollback_and_close().await;
        let mut error: anyhow::Error = error.into();
        if let Err(cleanup_error) = cleanup_error {
            error = error.context(format!(
                "initial summary manifest rollback and close failed after commit error: {cleanup_error}"
            ));
        }
        return Err(error);
    }
    resources.mark_transaction_finished();
    #[cfg(test)]
    resources.observe_return_for_test("archive_snapshot_manifest_stage");
    let archive_lock = resources
        .return_to_pool_retaining_archive_fence()
        .await
        .context("return initial summary manifest connection to pool")?;
    drop(archive_lock);
    Ok(Some(batch_id))
}

pub(crate) async fn clear_task_summary_snapshot_pages(
    pool: &Pool<Sqlite>,
    batch_id: i64,
    manifest_sha256: &str,
    observed_pages: &[TaskArchiveSnapshotPageFingerprint],
) -> Result<Option<()>> {
    let Some((connection, admission)) =
        acquire_retention_batch_write_connection(pool, "archive_snapshot_page_reset").await?
    else {
        return Ok(None);
    };
    let mut resources = RetentionTransactionResources::without_archive_fence(connection, admission);
    resources.mark_transaction_started();
    if let Err(error) = sqlx::query("BEGIN IMMEDIATE")
        .execute(resources.connection_mut())
        .await
    {
        let cleanup_error = resources.rollback_and_close().await;
        let mut error: anyhow::Error = error.into();
        if let Err(cleanup_error) = cleanup_error {
            error = error.context(format!(
                "failed to rollback and close invalid summary page reset: {cleanup_error}"
            ));
        }
        return Err(error);
    }
    let reset_result: Result<Option<()>> = async {
        let manifest_matches = sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(
                SELECT 1 FROM archive_batches
                WHERE id=?1 AND sha256=?2
            )",
        )
        .bind(batch_id)
        .bind(manifest_sha256)
        .fetch_one(resources.connection_mut())
        .await?;
        if manifest_matches == 0 {
            bail!("summary archive manifest changed before page reset");
        }
        let current_pages = sqlx::query_as::<_, (i64, String, String, i64, Vec<u8>, i64, i64)>(
            "SELECT page_index, coverage_start, coverage_end, row_count, payload,
                    payload_bytes, format_version
             FROM summary_archive_snapshot
             WHERE archive_batch_id=?1 AND manifest_sha256=?2
             ORDER BY page_index ASC",
        )
        .bind(batch_id)
        .bind(manifest_sha256)
        .fetch_all(resources.connection_mut())
        .await?
        .into_iter()
        .map(
            |(
                page_index,
                coverage_start,
                coverage_end,
                row_count,
                payload,
                payload_bytes,
                format_version,
            )| TaskArchiveSnapshotPageFingerprint {
                page_index,
                coverage_start,
                coverage_end,
                row_count,
                payload_bytes,
                payload_sha256:
                    crate::summary_source_change::summary_archive_snapshot_payload_sha256(&payload),
                format_version,
            },
        )
        .collect::<Vec<_>>();
        let proof_exists = sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(
                SELECT 1 FROM summary_archive_snapshot_v2_proof
                WHERE archive_batch_id=?1 AND manifest_sha256=?2
            )",
        )
        .bind(batch_id)
        .bind(manifest_sha256)
        .fetch_one(resources.connection_mut())
        .await?;
        // A competing writer may have repaired and proved the page set after the reader found
        // it invalid. Leave that newer state intact; the caller retries against the new prefix.
        if current_pages != observed_pages || proof_exists != 0 {
            return Ok(None);
        }
        sqlx::query(
            "DELETE FROM summary_archive_snapshot
             WHERE archive_batch_id=?1 AND manifest_sha256=?2",
        )
        .bind(batch_id)
        .bind(manifest_sha256)
        .execute(resources.connection_mut())
        .await?;
        Ok(Some(()))
    }
    .await;
    let reset = match reset_result {
        Ok(reset) => reset,
        Err(error) => {
            let cleanup_error = resources.rollback_and_close().await;
            let mut error = error;
            if let Err(cleanup_error) = cleanup_error {
                error = error.context(format!(
                    "invalid summary page reset rollback and close failed: {cleanup_error}"
                ));
            }
            return Err(error);
        }
    };
    if reset.is_none() {
        resources
            .rollback_and_close()
            .await
            .context("rollback raced summary page reset")?;
        return Ok(None);
    }
    if let Err(error) = sqlx::query("COMMIT")
        .execute(resources.connection_mut())
        .await
    {
        let cleanup_error = resources.rollback_and_close().await;
        let mut error: anyhow::Error = error.into();
        if let Err(cleanup_error) = cleanup_error {
            error = error.context(format!(
                "invalid summary page reset rollback and close failed after commit error: {cleanup_error}"
            ));
        }
        return Err(error);
    }
    resources.mark_transaction_finished();
    resources
        .return_to_pool()
        .await
        .context("return invalid summary page reset connection to pool")?;
    Ok(Some(()))
}

pub(crate) async fn store_task_summary_snapshot_page(
    pool: &Pool<Sqlite>,
    outcome: &ArchiveBatchOutcome,
    batch_id: i64,
    page_index: u32,
    page: TaskArchiveSnapshotPage,
) -> Result<Option<()>> {
    let Some((connection, admission)) =
        acquire_retention_batch_write_connection(pool, "archive_snapshot_page").await?
    else {
        return Ok(None);
    };
    let mut resources = RetentionTransactionResources::without_archive_fence(connection, admission);
    resources.mark_transaction_started();
    if let Err(error) = sqlx::query("BEGIN IMMEDIATE")
        .execute(resources.connection_mut())
        .await
    {
        let cleanup_error = resources.rollback_and_close().await;
        let mut error: anyhow::Error = error.into();
        if let Err(cleanup_error) = cleanup_error {
            error = error.context(format!(
                "failed to rollback and close summary page connection after begin failure: {cleanup_error}"
            ));
        }
        return Err(error);
    }
    let page_result: Result<()> = async {
        let manifest_matches = sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(
                SELECT 1 FROM archive_batches
                WHERE id=?1 AND file_path=?2 AND sha256=?3
            )",
        )
        .bind(batch_id)
        .bind(&outcome.file_path)
        .bind(&outcome.sha256)
        .fetch_one(resources.connection_mut())
        .await?;
        if manifest_matches == 0 {
            bail!("summary archive manifest changed before page staging");
        }
        let payload_bytes =
            i64::try_from(page.payload.len()).context("summary page byte count overflow")?;
        let payload_sha256 =
            crate::summary_source_change::summary_archive_snapshot_payload_sha256(&page.payload);
        let existing_page = sqlx::query_as::<_, (String, String, i64, Vec<u8>, i64, String, i64)>(
            "SELECT coverage_start, coverage_end, row_count, payload, payload_bytes,
                    snapshot_sha256, format_version
             FROM summary_archive_snapshot
             WHERE archive_batch_id=?1 AND manifest_sha256=?2 AND page_index=?3",
        )
        .bind(batch_id)
        .bind(&outcome.sha256)
        .bind(i64::from(page_index))
        .fetch_optional(resources.connection_mut())
        .await?;
        if existing_page.is_some_and(
            |(
                coverage_start,
                coverage_end,
                row_count,
                existing_payload,
                existing_payload_bytes,
                existing_payload_sha256,
                format_version,
            )| {
                coverage_start == page.coverage_start
                    && coverage_end == page.coverage_end
                    && row_count == i64::from(page.row_count)
                    && existing_payload == page.payload
                    && existing_payload_bytes == payload_bytes
                    && existing_payload_sha256 == payload_sha256
                    && format_version == SUMMARY_ARCHIVE_SNAPSHOT_V2
            },
        ) {
            // Replaying an identical page must not fire the page trigger that revokes an
            // already committed final proof for the same artifact identity.
            return Ok(());
        }
        sqlx::query(
            "DELETE FROM summary_archive_snapshot
             WHERE archive_batch_id=?1 AND manifest_sha256=?2 AND page_index=?3",
        )
        .bind(batch_id)
        .bind(&outcome.sha256)
        .bind(i64::from(page_index))
        .execute(resources.connection_mut())
        .await?;
        store_summary_archive_snapshot_page_v2_tx(
            resources.connection_mut(),
            &SummaryArchiveSnapshotPage {
                archive_batch_id: batch_id,
                manifest_sha256: outcome.sha256.clone(),
                page_index,
                coverage_start: page.coverage_start,
                coverage_end: page.coverage_end,
                row_count: page.row_count,
                payload: page.payload,
            },
        )
        .await
    }
    .await;
    if let Err(error) = page_result {
        let cleanup_error = resources.rollback_and_close().await;
        let mut error = error;
        if let Err(cleanup_error) = cleanup_error {
            error = error.context(format!(
                "summary page transaction rollback and close failed: {cleanup_error}"
            ));
        }
        return Err(error);
    }
    if let Err(error) = sqlx::query("COMMIT")
        .execute(resources.connection_mut())
        .await
    {
        let cleanup_error = resources.rollback_and_close().await;
        let mut error: anyhow::Error = error.into();
        if let Err(cleanup_error) = cleanup_error {
            error = error.context(format!(
                "summary page transaction rollback and close failed after commit failure: {cleanup_error}"
            ));
        }
        return Err(error);
    }
    resources.mark_transaction_finished();
    resources
        .return_to_pool()
        .await
        .context("return summary page transaction connection to pool")?;
    Ok(Some(()))
}

pub(super) async fn prepare_summary_proof(
    pool: &Pool<Sqlite>,
    outcome: &ArchiveBatchOutcome,
) -> Result<Option<VerifiedSummaryArchiveSnapshot>> {
    let archive_path = Path::new(&outcome.file_path);
    let Some((connection, admission, archive_lock)) =
        acquire_retention_batch_write_connection_with_archive_fence(
            pool,
            "archive_snapshot_manifest",
            archive_path,
            Some(&outcome.sha256),
        )
        .await?
    else {
        return Ok(None);
    };
    let mut transaction_resources =
        RetentionTransactionResources::new(connection, admission, archive_lock);
    #[cfg(test)]
    transaction_resources.observe_return_for_test("archive_snapshot_manifest");
    transaction_resources.mark_transaction_started();
    let begin_result = sqlx::query("BEGIN")
        .execute(transaction_resources.connection_mut())
        .await;
    if let Err(error) = begin_result {
        let cleanup_error = transaction_resources.rollback_and_close().await;
        let mut error: anyhow::Error = error.into();
        if let Err(cleanup_error) = cleanup_error {
            error = error.context(format!(
                "failed to rollback and close summary manifest connection after begin failure: {cleanup_error}"
            ));
        }
        return Err(error);
    }
    let batch_id_result: Result<i64> = async {
        stage_invocation_archive_batch_manifest(transaction_resources.connection_mut(), outcome)
            .await?;
        load_archive_batch_id_for_file_tx(
            transaction_resources.connection_mut(),
            outcome.dataset,
            &outcome.month_key,
            &outcome.file_path,
        )
        .await
    }
    .await;
    let batch_id = match batch_id_result {
        Ok(batch_id) => batch_id,
        Err(error) => {
            let cleanup_error = transaction_resources.rollback_and_close().await;
            let mut error = error;
            if let Err(cleanup_error) = cleanup_error {
                error = error.context(format!(
                    "summary manifest transaction rollback and close failed: {cleanup_error}"
                ));
            }
            return Err(error);
        }
    };
    if let Err(error) = sqlx::query("COMMIT")
        .execute(transaction_resources.connection_mut())
        .await
    {
        let cleanup_error = transaction_resources.rollback_and_close().await;
        let mut error: anyhow::Error = error.into();
        if let Err(cleanup_error) = cleanup_error {
            error = error.context(format!(
                "summary manifest transaction rollback and close failed after commit failure: {cleanup_error}"
            ));
        }
        return Err(error);
    }
    transaction_resources.mark_transaction_finished();
    let archive_lock = transaction_resources
        .return_to_pool_retaining_archive_fence()
        .await
        .context("return summary manifest transaction connection while retaining archive fence")?;
    // Summary pages are persisted by the monthly writer exactly once. This phase only validates
    // the published artifact and the already durable page set before granting deletion authority.
    let proof = prepare_verified_summary_archive_snapshot(pool, batch_id, &outcome.sha256).await?;
    if persist_summary_proof(pool, &proof).await?.is_none() {
        drop(archive_lock);
        return Ok(None);
    }
    drop(archive_lock);
    Ok(Some(proof))
}

async fn persist_summary_proof(
    pool: &Pool<Sqlite>,
    proof: &crate::VerifiedSummaryArchiveSnapshot,
) -> Result<Option<()>> {
    let Some((connection, admission)) =
        acquire_retention_batch_write_connection(pool, "archive_snapshot_proof").await?
    else {
        return Ok(None);
    };
    let mut resources = RetentionTransactionResources::without_archive_fence(connection, admission);
    resources.mark_transaction_started();
    let result: Result<()> = async {
        sqlx::query("BEGIN IMMEDIATE")
            .execute(resources.connection_mut())
            .await?;
        crate::store_verified_summary_archive_snapshot_tx(resources.connection_mut(), proof)
            .await?;
        sqlx::query("COMMIT")
            .execute(resources.connection_mut())
            .await?;
        Ok(())
    }
    .await;
    if let Err(error) = result {
        let cleanup_error = resources.rollback_and_close().await;
        if let Err(cleanup_error) = cleanup_error {
            return Err(error.context(format!(
                "summary proof transaction rollback and close failed: {cleanup_error}"
            )));
        }
        return Err(error);
    }
    resources.mark_transaction_finished();
    resources
        .return_to_pool()
        .await
        .context("return summary proof transaction connection to pool")?;
    Ok(Some(()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_file_batch_is_independent_of_microtransaction_limit() {
        let rows = select_archive_batch((0..2_000).collect(), |_| 256);
        assert_eq!(rows.len(), 1_000);
        assert!(rows.len() > RETENTION_WRITE_MAX_ROWS);
    }

    #[test]
    fn archive_file_candidate_limit_is_independent_of_configured_write_rows() {
        let mut config = crate::tests::test_config();
        config.retention_batch_rows = 2;
        assert_eq!(archive_candidate_limit(&config), ARCHIVE_BATCH_TARGET_ROWS);
    }

    #[test]
    fn archive_file_batch_preserves_tail_and_payload_boundary() {
        assert_eq!(select_archive_batch(vec![1, 2, 3], |_| 256), vec![1, 2, 3]);
        assert_eq!(
            select_archive_batch(vec![1, 2, 3], |_| 9 * 1024 * 1024),
            vec![1]
        );
    }

    #[tokio::test]
    async fn retry_window_limit_preserves_the_original_selected_scope() {
        let config = crate::tests::test_config();
        TASK_BATCH_METRICS
            .scope(
                RefCell::new(Vec::new()),
                TASK_RETRY_WINDOW_REMAINING.scope(RefCell::new(None), async {
                    let mut observation = BatchObservation::begin(
                        "pool_upstream_request_attempts",
                        "2026-08",
                        ARCHIVE_BATCH_TARGET_ROWS,
                    );
                    observation.committed(64, Duration::ZERO);
                    observation.complete();
                    drop(observation);

                    retain_committed_batch_prefixes_for_retry();

                    assert_eq!(
                        TASK_RETRY_WINDOW_REMAINING.with(|remaining| *remaining.borrow()),
                        Some(936)
                    );
                    assert_eq!(archive_candidate_limit(&config), 936);
                    let metrics = TASK_BATCH_METRICS.with(|metrics| metrics.borrow().clone());
                    assert_eq!(metrics.len(), 1);
                    assert_eq!(metrics[0].batch_rows, 64);
                    assert_eq!(metrics[0].committed_rows, 64);
                }),
            )
            .await;
    }
}
