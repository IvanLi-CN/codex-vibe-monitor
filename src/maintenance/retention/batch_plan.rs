use super::*;

const ARCHIVE_BATCH_TARGET_ROWS: usize = 1_000;
const ARCHIVE_BATCH_MAX_BYTES: usize = 16 * 1024 * 1024;
const ARCHIVE_BATCH_SETTLEMENT_RESERVE: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub(crate) struct TaskArchiveSnapshotPage {
    pub(crate) coverage_start: String,
    pub(crate) coverage_end: String,
    pub(crate) row_count: u32,
    pub(crate) payload: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RetentionBatchMetrics {
    pub(crate) dataset: String,
    pub(crate) month_key: String,
    pub(crate) batch_rows: usize,
    pub(crate) committed_rows: usize,
    pub(crate) file_prepare_ms: u64,
    pub(crate) lock_wait_ms: u64,
    pub(crate) elapsed_ms: u64,
    pub(crate) committed_rows_per_second: Option<f64>,
    pub(crate) arrival_rows_per_second: Option<f64>,
    pub(crate) service_rate_multiple: Option<f64>,
    pub(crate) small_batch_reason: Option<String>,
}

tokio::task_local! {
    pub(super) static TASK_BATCH_METRICS: RefCell<Vec<RetentionBatchMetrics>>;
}

pub(crate) struct BatchObservation {
    metrics: RetentionBatchMetrics,
    started: Instant,
}

impl BatchObservation {
    pub(crate) fn begin(dataset: &str, month: &str, rows: usize) -> Self {
        Self {
            started: Instant::now(),
            metrics: RetentionBatchMetrics {
                dataset: dataset.to_string(),
                month_key: month.to_string(),
                batch_rows: rows,
                committed_rows: 0,
                file_prepare_ms: 0,
                lock_wait_ms: 0,
                elapsed_ms: 0,
                committed_rows_per_second: None,
                arrival_rows_per_second: None,
                service_rate_multiple: None,
                small_batch_reason: (rows < 512)
                    .then(|| "tail_config_or_payload_limit".to_string()),
            },
        }
    }
    pub(crate) fn file_prepared(&mut self, elapsed: Duration) {
        self.metrics.file_prepare_ms = elapsed.as_millis() as u64;
    }
    pub(crate) fn committed(&mut self, rows: usize, lock_wait: Duration) {
        self.metrics.committed_rows += rows;
        self.metrics.lock_wait_ms = self
            .metrics
            .lock_wait_ms
            .saturating_add(lock_wait.as_millis() as u64);
    }
}

impl Drop for BatchObservation {
    fn drop(&mut self) {
        self.metrics.elapsed_ms = self.started.elapsed().as_millis() as u64;
        let _ =
            TASK_BATCH_METRICS.try_with(|metrics| metrics.borrow_mut().push(self.metrics.clone()));
    }
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
pub(crate) fn archive_candidate_limit(config: &AppConfig) -> usize {
    config
        .retention_batch_rows
        .clamp(1, ARCHIVE_BATCH_TARGET_ROWS)
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

pub(super) async fn prepare_summary_proof(
    pool: &Pool<Sqlite>,
    outcome: &ArchiveBatchOutcome,
    pages: Vec<TaskArchiveSnapshotPage>,
) -> Result<Option<VerifiedSummaryArchiveSnapshot>> {
    let Some(admission) = acquire_retention_write_admission("archive_snapshot_manifest").await
    else {
        return Ok(None);
    };
    let mut transaction = pool.begin().await?;
    stage_invocation_archive_batch_manifest(transaction.as_mut(), outcome).await?;
    let batch_id = load_archive_batch_id_for_file_tx(
        transaction.as_mut(),
        outcome.dataset,
        &outcome.month_key,
        &outcome.file_path,
    )
    .await?;
    transaction.commit().await?;
    drop(admission);
    for (index, page) in pages.into_iter().enumerate() {
        let Some(admission) = acquire_retention_write_admission("archive_snapshot_page").await
        else {
            return Ok(None);
        };
        let mut transaction = pool.begin().await?;
        // Older digests are no longer the identity of the canonical file. Replace only this
        // page, keeping each write short and preventing one full copy per task from piling up.
        sqlx::query(
            "DELETE FROM summary_archive_snapshot WHERE archive_batch_id=?1 AND page_index=?2",
        )
        .bind(batch_id)
        .bind(index as i64)
        .execute(transaction.as_mut())
        .await?;
        store_summary_archive_snapshot_page_v2_tx(
            transaction.as_mut(),
            &SummaryArchiveSnapshotPage {
                archive_batch_id: batch_id,
                manifest_sha256: outcome.sha256.clone(),
                page_index: u32::try_from(index)?,
                coverage_start: page.coverage_start,
                coverage_end: page.coverage_end,
                row_count: page.row_count,
                payload: page.payload,
            },
        )
        .await?;
        transaction.commit().await?;
        drop(admission);
    }
    // Pages are not deletion authority until the complete semantic proof is checked. Reads
    // and decompression of the month happen with no main-database writer permit held.
    Ok(Some(
        prepare_verified_summary_archive_snapshot(pool, batch_id, &outcome.sha256).await?,
    ))
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
    fn archive_file_batch_preserves_tail_and_payload_boundary() {
        assert_eq!(select_archive_batch(vec![1, 2, 3], |_| 256), vec![1, 2, 3]);
        assert_eq!(
            select_archive_batch(vec![1, 2, 3], |_| 9 * 1024 * 1024),
            vec![1]
        );
    }
}
